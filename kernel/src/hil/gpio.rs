use crate::common::cells::OptionalCell;
use core::cell::Cell;

/// Enum for configuring any pull-up or pull-down resistors on the GPIO pin.
#[derive(Debug)]
pub enum FloatingState {
    PullUp,
    PullDown,
    PullNone,
}

/// Enum for selecting which edge to trigger interrupts on.
#[derive(Debug)]
pub enum InterruptEdge {
    RisingEdge,
    FallingEdge,
    EitherEdge,
}

/// Enum for which state the pin is in. Some MCUs can support Input/Output pins,
/// so this is a valid option. `Function` means the pin has been configured to
/// a special function. Determining which function it outside the scope of the HIL,
/// and should instead use a chip-specific API.
#[derive(Debug)]
pub enum Configuration {
    LowPower,
    Input,
    Output,
    InputOutput,
    Function, // Chip-specific, requires chip-specific API for more detail
    Unknown,
}

pub trait Pin: Input + Output + Configure {}
pub trait InterruptPin: Pin + Interrupt {}

pub trait Configure {
    fn configuration(&self) -> Configuration;

    fn make_output(&self) -> Configuration;
    fn disable_output(&self) -> Configuration;
    fn make_input(&self) -> Configuration;
    fn disable_input(&self) -> Configuration;

    // Disable the pin and put it into its lowest power state.
    // Re-enabling the pin requires reconfiguring it (state of
    // its enabled configuration is not stored).
    fn low_power(&self);

    fn set_floating_state(&self, state: FloatingState);
    fn floating_state(&self) -> FloatingState;

    fn is_input(&self) -> bool;
    fn is_output(&self) -> bool;
}

pub trait Output {
    /// Set the GPIO pin high. If the pin is not an output or
    /// input/output, this call is ignored.
    fn set(&self);

    /// Set the GPIO pin low. If the pin is not an output or
    /// input/output, this call is ignored.
    fn clear(&self);

    /// Toggle the GPIO pin. If the pin was high, set it low. If
    /// the pin was low, set it high. If the pin is not an output or
    /// input/output, this call is ignored. Return the new value
    /// of the pin.
    fn toggle(&self) -> bool;
}

pub trait Input {
    /// Get the current state of an input GPIO pin. For an output
    /// pin, return the output; for an input pin, return the input;
    /// for disabled or function pins the value is undefined.
    fn read(&self) -> bool;
}

pub trait Interrupt: Input {
    /// Set the client for interrupt events.
    fn set_client(&self, client: &'static Client);
    
    /// Enable an interrupt on the GPIO pin. This does not
    /// configure the pin except to enable an interrupt: it
    /// should be separately configured as an input, etc.
    fn enable_interrupts(&self, mode: InterruptEdge);

    /// Disable interrupts for the GPIO pin.
    fn disable_interrupts(&self);
 
    /// Return whether this interrupt is pending
    fn is_pending(&self) -> bool;
}

/// Interface for users of synchronous GPIO interrupts. In order
/// to receive interrupts, the user must implement
/// this `Client` interface.
pub trait Client {
    /// Called when an interrupt occurs. The `identifier` will
    /// be the same value that was passed to `enable_interrupt()`
    /// when the interrupt was configured.
    fn fired(&self);
}

/// Interfaces for users of GPIO interrupts who handle many interrupts
/// with the same function. The value passed in the callback allows the
/// callback to distinguish which interrupt fired. 
pub trait ClientWithValue {
    fn fired(&self, value: u32);
}

pub struct InterruptWithValue {
    value: Cell<u32>,
    client: OptionalCell<&'static ClientWithValue>,
}

impl InterruptWithValue {
    pub fn new() -> InterruptWithValue {
        InterruptWithValue {
            value: Cell::new(0),
            client: OptionalCell::empty(),
        }
    }

   pub fn set_value(&self, value: u32) {
        self.value.set(value);
   }

   pub fn value(&self) -> u32 {
        self.value.get()
   }

   pub fn set_client(&self, client: &'static ClientWithValue) {
        self.client.replace(client);
   }
}

impl Client for InterruptWithValue {
    fn fired(&self) {
        self.client.map(|c| c.fired(self.value()));
    }
}
