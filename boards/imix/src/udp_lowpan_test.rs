//! `udp_lowpan_test.rs`: Kernel test suite for the UDP/6LoWPAN stack
//!
//! This file tests port binding and sending messages from kernel space.
//! It instantiates two capsules that use the UDP stack, and tests various
//! binding and sending orders to ensure that port binding and sending
//! is enforced as expected.
//! The messages sent are long enough to require multiple fragments). The payload of each message
//! is all 0's. Tests for UDP reception exist in userspace, but not in the kernel
//! at this point in time. At the conclusion of the test, it prints "Test completed successfully."
//!
//! To use this test suite, insert the code into `boards/imix/src/main.rs` as follows:
//!
//! ...
//! // Radio initialization code
//! ...
//!    let udp_lowpan_test = udp_lowpan_test::initialize_all(
//!        udp_mux,
//!        mux_alarm as &'static MuxAlarm<'static, sam4l::ast::Ast>,
//!    );
//! ...
//! // Imix initialization
//! ...
//! udp_lowpan_test.start();

use super::components::mock_udp::MockUDPComponent;
use super::components::mock_udp2::MockUDPComponent2;
use capsules::ieee802154::device::MacDevice;
use capsules::mock_udp::MockUdp1;
use capsules::net::buffer::Buffer;
use capsules::net::ieee802154::MacAddress;
use capsules::net::ipv6::ip_utils::{ip6_nh, IPAddr};
use capsules::net::ipv6::ipv6::{IP6Header, IP6Packet, IPPayload, TransportHeader};
use capsules::net::ipv6::ipv6_send::{IP6SendStruct, IP6Sender};
use capsules::net::sixlowpan::sixlowpan_compression;
use capsules::net::sixlowpan::sixlowpan_state::{Sixlowpan, SixlowpanState, TxState};
use capsules::net::udp::udp::UDPHeader;
use capsules::net::udp::udp_recv::MuxUdpReceiver;
use capsules::net::udp::udp_send::{MuxUdpSender, UDPSendStruct, UDPSender};
use capsules::virtual_alarm::{MuxAlarm, VirtualMuxAlarm};
use core::cell::Cell;
use kernel::common::cells::MapCell;
use kernel::component::Component;
use kernel::debug;
use kernel::hil::radio;
use kernel::hil::time;
use kernel::hil::time::Frequency;
use kernel::static_init;
use kernel::udp_port_table::UdpPortTable;
use kernel::ReturnCode;

pub const TEST_DELAY_MS: u32 = 2000;
pub const TEST_LOOP: bool = false;

const UDP_HDR_SIZE: usize = 8;
const PAYLOAD_LEN: usize = super::components::udp_mux::PAYLOAD_LEN;
static mut UDP_PAYLOAD1: [u8; PAYLOAD_LEN - UDP_HDR_SIZE] = [0; PAYLOAD_LEN - UDP_HDR_SIZE];
static mut UDP_PAYLOAD2: [u8; PAYLOAD_LEN - UDP_HDR_SIZE] = [0; PAYLOAD_LEN - UDP_HDR_SIZE];

//Use a global variable option, initialize as None, then actually initialize in initialize all

pub struct LowpanTest<'a, A: time::Alarm> {
    alarm: &'a A,
    test_counter: Cell<usize>,
    port_table: &'static UdpPortTable,
    mock_udp1: &'a MockUdp1<'a, A>,
    mock_udp2: &'a MockUdp1<'a, A>,
}
//TODO: Initialize UDP sender/send_done client in initialize all
pub unsafe fn initialize_all(
    udp_send_mux: &'static MuxUdpSender<
        'static,
        IP6SendStruct<'static, VirtualMuxAlarm<'static, sam4l::ast::Ast<'static>>>,
    >,
    udp_recv_mux: &'static MuxUdpReceiver<'static>,
    port_table: &'static UdpPortTable,
    mux_alarm: &'static MuxAlarm<'static, sam4l::ast::Ast>,
) -> &'static LowpanTest<
    'static,
    capsules::virtual_alarm::VirtualMuxAlarm<'static, sam4l::ast::Ast<'static>>,
> {
    let mock_udp1 = MockUDPComponent::new(
        udp_send_mux,
        udp_recv_mux,
        port_table,
        mux_alarm,
        &mut UDP_PAYLOAD1,
        1, //id
        3, //dst_port
    )
    .finalize();

    let mock_udp2 = MockUDPComponent2::new(
        udp_send_mux,
        udp_recv_mux,
        port_table,
        mux_alarm,
        &mut UDP_PAYLOAD2,
        2, //id
        4, //dst_port
    )
    .finalize();

    let udp_lowpan_test = static_init!(
        LowpanTest<'static, VirtualMuxAlarm<'static, sam4l::ast::Ast>>,
        LowpanTest::new(
            static_init!(
                VirtualMuxAlarm<'static, sam4l::ast::Ast>,
                VirtualMuxAlarm::new(mux_alarm)
            ),
            port_table,
            mock_udp1,
            mock_udp2
        )
    );

    udp_lowpan_test.alarm.set_client(udp_lowpan_test);

    udp_lowpan_test
}

impl<'a, A: time::Alarm> LowpanTest<'a, A> {
    pub fn new(
        alarm: &'a A,
        port_table: &'static UdpPortTable,
        mock_udp1: &'static MockUdp1<'a, A>,
        mock_udp2: &'static MockUdp1<'a, A>,
    ) -> LowpanTest<'a, A> {
        LowpanTest {
            alarm: alarm,
            test_counter: Cell::new(0),
            port_table: port_table,
            mock_udp1: mock_udp1,
            mock_udp2: mock_udp2,
        }
    }

    pub fn start(&self) {
        self.schedule_next();
    }

    fn schedule_next(&self) {
        let delta = (A::Frequency::frequency() * TEST_DELAY_MS) / 1000;
        let next = self.alarm.now().wrapping_add(delta);
        self.alarm.set_alarm(next);
    }

    fn run_test_and_increment(&self) {
        let test_counter = self.test_counter.get();
        self.run_test(test_counter);
        match TEST_LOOP {
            true => self.test_counter.set((test_counter + 1) % self.num_tests()),
            false => self.test_counter.set(test_counter + 1),
        };
    }

    fn num_tests(&self) -> usize {
        2
    }

    fn run_test(&self, test_id: usize) {
        debug!("Running test {}:", test_id);
        match test_id {
            0 => self.port_table_test(),
            1 => self.capsule_send_test(),
            _ => return,
        }
        self.schedule_next();
    }

    // A basic test of port table functionality without using any capsules at all,
    // instead directly creating socket and calling bind/unbind.
    // This test ensures that two capsules could not bind to the same port,
    // that single bindings work correctly,
    fn port_table_test(&self) {
        // Initialize bindings.
        let socket1 = self.port_table.create_socket().unwrap();
        let mut socket2 = self.port_table.create_socket().unwrap();
        let socket3 = self.port_table.create_socket().unwrap();
        //debug!("Finished creating sockets");
        // Attempt to bind to a port that has already been bound.
        let (send_bind, recv_bind) = self.port_table.bind(socket1, 4000).expect("fail1");
        let result = self.port_table.bind(socket2, 4000);
        assert!(result.is_err());
        socket2 = result.unwrap_err();
        let (send_bind2, recv_bind2) = self.port_table.bind(socket2, 4001).expect("fail2");

        // Ensure that only the first binding is able to send
        assert_eq!(send_bind.get_port(), 4000);
        assert_eq!(recv_bind.get_port(), 4000);
        assert!(self.port_table.unbind(send_bind, recv_bind).is_ok());

        // Show that you can bind to a port once another socket has unbound it
        let (send_bind3, recv_bind3) = self.port_table.bind(socket3, 4000).expect("fail3");
        assert!(self.port_table.unbind(send_bind3, recv_bind3).is_ok());

        debug!("port_table_test passed");
    }

    fn capsule_send_test(&self) {
        self.mock_udp1.bind(14000);
        self.mock_udp1.set_dst(15000);
        self.mock_udp2.bind(14001);
        self.mock_udp2.set_dst(15001);
        // Send from 2 different capsules in quick succession - second send should execute once
        // first completes!
        self.mock_udp1.send(22);
        self.mock_udp2.send(23);
    }
}

impl<'a, A: time::Alarm> time::Client for LowpanTest<'a, A> {
    fn fired(&self) {
        self.run_test_and_increment();
    }
}
