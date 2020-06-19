//! Provides userspace applications with a alarm API.

use core::cell::Cell;
use kernel::hil::time::{self, Alarm, Frequency, Ticks};
use kernel::{AppId, Callback, Driver, Grant, ReturnCode};

/// Syscall driver number.
use crate::driver;
pub const DRIVER_NUM: usize = driver::NUM::Alarm as usize;

// This should transition to using Ticks
#[derive(Copy, Clone, Debug)]
enum Expiration {
    Disabled,
    Enabled(u32, u32), // reference, dt
}

#[derive(Copy, Clone)]
pub struct AlarmData {
    expiration: Expiration,
    callback: Option<Callback>,
}

impl Default for AlarmData {
    fn default() -> AlarmData {
        AlarmData {
            expiration: Expiration::Disabled,
            callback: None,
        }
    }
}

pub struct AlarmDriver<'a, A: Alarm<'a>> {
    alarm: &'a A,
    num_armed: Cell<usize>,
    app_alarms: Grant<AlarmData>,
    next_alarm: Cell<Expiration>
}

impl<A: Alarm<'a>> AlarmDriver<'a, A> {
    pub const fn new(alarm: &'a A, grant: Grant<AlarmData>) -> AlarmDriver<'a, A> {
        AlarmDriver {
            alarm: alarm,
            num_armed: Cell::new(0),
            app_alarms: grant,
            next_alarm: Cell::new(Expiration::Disabled),
        }
    }

    fn reset_active_alarm(&self) {
        let mut earliest_alarm = Expiration::Disabled;
        let mut earliest_end: A::Ticks = A::Ticks::from(0);
        let now = self.alarm.now();
        // Find the first alarm to fir and store it in earliest_alarm,
        // its counter value at earliest_end.
        for alarm in self.app_alarms.iter() {
            alarm.enter(|alarm, _| match alarm.expiration {
                Expiration::Enabled(reference, dt) => {
                    let end: A::Ticks = A::Ticks::from(reference.wrapping_add(dt));
                    earliest_alarm = match earliest_alarm {
                        Expiration::Disabled => {
                            earliest_end = end;
                            alarm.expiration
                        },
                        Expiration::Enabled(earliest_reference, _) => {
                            // There are two cases when this might be
                            // an earlier alarm.  The first is if it
                            // fires inside the interval (reference,
                            // reference+dt) of the existing earliest.
                            // The second is if now is not within the
                            // interval: this means that it has
                            // passed. It could be the earliest has passed
                            // too, but at this point we don't need to track
                            // which is earlier: the key point is that
                            // the alarm must fire immediately, and then when
                            // we handle the alarm callback we will handle the
                            // two in the correct order.
                            if end.within_range(A::Ticks::from(earliest_reference), earliest_end) {
                                earliest_end = end;
                                alarm.expiration
                            } else if !now.within_range(A::Ticks::from(reference), A::Ticks::from(dt)) {
                                earliest_end = end;
                                alarm.expiration
                            } else {
                                earliest_alarm
                            }
                        }
                    }
                }, 
                Expiration::Disabled => {}
            });
        }
        self.next_alarm.set(earliest_alarm);
        match earliest_alarm {
            Expiration::Disabled => {
                self.alarm.disarm();
            },
            Expiration::Enabled(reference, dt) => {
                self.alarm.set_alarm(A::Ticks::from(reference), A::Ticks::from(dt));
            }
        }
    }
}

impl<A: Alarm<'a>> Driver for AlarmDriver<'a, A> {
    /// Subscribe to alarm expiration
    ///
    /// ### `_subscribe_num`
    ///
    /// - `0`: Subscribe to alarm expiration
    fn subscribe(
        &self,
        _subscribe_num: usize,
        callback: Option<Callback>,
        app_id: AppId,
    ) -> ReturnCode {
        self.app_alarms
            .enter(app_id, |td, _allocator| {
                td.callback = callback;
                ReturnCode::SUCCESS
            })
            .unwrap_or_else(|err| err.into())
    }

    /// Setup and read the alarm.
    ///
    /// ### `command_num`
    ///
    /// - `0`: Driver check.
    /// - `1`: Return the clock frequency in Hz.
    /// - `2`: Read the the current clock value
    /// - `3`: Stop the alarm if it is outstanding
    /// - `4`: Set an alarm to fire at a given clock value `time`.
    fn command(&self, cmd_type: usize, data: usize, data2: usize, caller_id: AppId) -> ReturnCode {
        // Returns the error code to return to the user and whether we need to
        // reset which is the next active alarm. We only _don't_ reset if we're
        // disabling the underlying alarm anyway, if the underlying alarm is
        // currently disabled and we're enabling the first alarm, or on an error
        // (i.e. no change to the alarms).
        self.app_alarms
            .enter(caller_id, |td, _alloc| {
                let now = self.alarm.now();
                let (return_code, reset) = match cmd_type {
                    0 /* check if present */ => (ReturnCode::SuccessWithValue { value: 1 }, false),
                    1 /* Get clock frequency */ => {
                        let freq = <A::Frequency>::frequency() as usize;
                        (ReturnCode::SuccessWithValue { value: freq }, false)
                    },
                    2 /* capture time */ => {
                        (ReturnCode::SuccessWithValue { value: now.into_u32() as usize },
                         false)
                    },
                    3 /* Stop */ => {
                        match td.expiration {
                            Expiration::Disabled => {
                                // Request to stop when already stopped
                                (ReturnCode::EALREADY, false)
                            },
                            _ => {
                                td.expiration = Expiration::Disabled;
                                let new_num_armed = self.num_armed.get() - 1;
                                self.num_armed.set(new_num_armed);
                                (ReturnCode::SUCCESS, true)
                            }
                        }
                    },
                    4 /* Set absolute expiration */ => {
                        let reference = data;
                        let dt = data2;
                        // if previously unarmed, but now will become armed
                        if let Expiration::Disabled = td.expiration {
                            self.num_armed.set(self.num_armed.get() + 1);
                        }
                        td.expiration = Expiration::Enabled(reference as u32, dt as u32);
                        (ReturnCode::SuccessWithValue { value: reference.wrapping_add(dt) }, true)
                    },
                    _ => (ReturnCode::ENOSUPPORT, false)
                };
                if reset {
                    self.reset_active_alarm();
                }
                return_code
            })
            .unwrap_or_else(|err| err.into())
    }
}

impl<A: Alarm<'a>> time::AlarmClient for AlarmDriver<'a, A> {
    fn alarm(&self) {
        let now: A::Ticks  = self.alarm.now();
        self.app_alarms.each(|alarm| {
            if let Expiration::Enabled(reference, ticks) = alarm.expiration {
                // Now is not within reference, reference + ticks; this timer
                // as passed (since reference must be in the past)
                if !now.within_range(A::Ticks::from(reference), A::Ticks::from(reference.wrapping_add(ticks))) {
                    alarm.expiration = Expiration::Disabled;
                    self.num_armed.set(self.num_armed.get() - 1);
                    alarm
                        .callback
                        .map(|mut cb| cb.schedule(now.into_u32() as usize, now.into_u32() as usize, 0));
                }
            }
        });

        // If there are armed alarms left, reset the underlying alarm to the
        // nearest interval.  Otherwise, disable the underlying alarm.
        if self.num_armed.get() == 0 {
            self.alarm.disarm();
        } else {
            self.reset_active_alarm();
            match self.next_alarm.get() {
                Expiration::Enabled(reference, dt) => {
                    let new_now: A::Ticks = self.alarm.now();
                    let ref_ticks = A::Ticks::from(reference);
                    let end_ticks = ref_ticks.wrapping_add(A::Ticks::from(dt));
                    if new_now.within_range(ref_ticks, end_ticks) {
                        self.alarm();
                    }
                },
                Expiration::Disabled => {
                    self.alarm.disarm();
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    pub fn alarm_before_systick_wrap_expired() {
        assert_eq!(super::has_expired(2u32, 3u32, 1u32), true);
    }

    #[test]
    pub fn alarm_before_systick_wrap_not_expired() {
        assert_eq!(super::has_expired(3u32, 2u32, 1u32), false);
    }

    #[test]
    pub fn alarm_after_systick_wrap_expired() {
        assert_eq!(super::has_expired(1u32, 2u32, 3u32), true);
    }

    #[test]
    pub fn alarm_after_systick_wrap_time_before_systick_wrap_not_expired() {
        assert_eq!(super::has_expired(1u32, 4u32, 3u32), false);
    }

    #[test]
    pub fn alarm_after_systick_wrap_time_after_systick_wrap_not_expired() {
        assert_eq!(super::has_expired(1u32, 0u32, 3u32), false);
    }
}
