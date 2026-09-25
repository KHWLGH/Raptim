//! Refresh-paced edge animation. No DWM wait runs on the UI thread and no
//! timer keeps waking a settled/hidden window. At most one frame is queued.
use super::{Notify, OwnedHandle};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::Duration,
};
use windows::Win32::{
    Foundation::{HANDLE, HWND, WAIT_OBJECT_0},
    Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo},
    System::{
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        Threading::*,
    },
};

/// Negative relative timer due time, in 100 ns units. Reading DWM's timing does
/// not wait for a present (fully off-screen HWNDs can throttle DwmFlush for 300 ms).
fn next_frame_due() -> i64 {
    let mut info = DWM_TIMING_INFO {
        cbSize: size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    let (mut now, mut frequency) = (0, 0);
    unsafe {
        if DwmGetCompositionTimingInfo(HWND::default(), &mut info).is_ok()
            && QueryPerformanceCounter(&mut now).is_ok()
            && QueryPerformanceFrequency(&mut frequency).is_ok()
            && frequency > 0
            && info.qpcRefreshPeriod > 0
        {
            let period = info.qpcRefreshPeriod as i64;
            let ticks = period - (now - info.qpcVBlank as i64).rem_euclid(period);
            // Leave enough time to dispatch, and bound stale/remote timing data.
            return -((ticks as f64 * 10_000_000. / frequency as f64) as i64).clamp(5_000, 200_000);
        }
    }
    -80_000
}

#[derive(Default)]
struct State {
    active: bool,
    stopped: bool,
    pending: bool,
    generation: u64,
}
impl State {
    fn set_active(&mut self, active: bool) {
        if self.active != active {
            self.active = active;
            self.pending = false;
            self.generation = self.generation.wrapping_add(1);
        }
    }
    fn acknowledge(&mut self, generation: u64) -> bool {
        if !self.active || !self.pending || self.generation != generation {
            return false;
        }
        self.pending = false;
        true
    }
}

pub struct FrameClock {
    state: Arc<(Mutex<State>, Condvar)>,
    worker: Option<JoinHandle<()>>,
    changed: OwnedHandle,
    _timer: OwnedHandle,
}
impl FrameClock {
    pub fn new(notify: Notify) -> Result<Self, String> {
        let timer = OwnedHandle(
            unsafe {
                CreateWaitableTimerExW(
                    None,
                    None,
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS.0,
                )
                .or_else(|_| CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0))
            }
            .map_err(|e| e.to_string())?,
        );
        let changed = OwnedHandle(
            unsafe { CreateEventW(None, false, false, None) }.map_err(|e| e.to_string())?,
        );
        let timer_handle = timer.0.0 as usize;
        let changed_handle = changed.0.0 as usize;
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let shared = state.clone();
        let worker = std::thread::Builder::new()
            .name("edge-frame-clock".into())
            .spawn(move || {
                let (lock, wake) = &*shared;
                let timer = HANDLE(timer_handle as _);
                let changed = HANDLE(changed_handle as _);
                loop {
                    let mut state = lock.lock().unwrap();
                    while !state.stopped && (!state.active || state.pending) {
                        state = wake.wait(state).unwrap();
                    }
                    if state.stopped {
                        break;
                    }
                    let generation = state.generation;
                    drop(state);
                    let armed =
                        unsafe { SetWaitableTimer(timer, &next_frame_due(), 0, None, None, false) }
                            .is_ok();
                    let mut signaled = false;
                    if armed {
                        let result =
                            unsafe { WaitForMultipleObjects(&[changed, timer], false, INFINITE) };
                        if result.0 == WAIT_OBJECT_0.0 {
                            continue;
                        }
                        signaled = result.0 == WAIT_OBJECT_0.0 + 1;
                    }
                    let mut state = lock.lock().unwrap();
                    if !signaled {
                        state = wake
                            .wait_timeout_while(state, Duration::from_millis(8), |s| {
                                !s.stopped && s.active && s.generation == generation
                            })
                            .unwrap()
                            .0;
                    }
                    if state.stopped {
                        break;
                    }
                    if !state.active || state.generation != generation || state.pending {
                        continue;
                    }
                    state.pending = true;
                    drop(state);
                    notify(11, generation.to_string());
                }
            })
            .map_err(|e| format!("无法创建动画时钟：{e}"))?;
        Ok(Self {
            state,
            worker: Some(worker),
            changed,
            _timer: timer,
        })
    }
    pub fn set_active(&self, active: bool) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap();
        let changed = state.active != active;
        state.set_active(active);
        if changed {
            unsafe {
                let _ = SetEvent(self.changed.0);
            }
            wake.notify_all();
        }
    }
    pub fn acknowledge(&self, generation: u64) -> bool {
        let (lock, wake) = &*self.state;
        let accepted = lock.lock().unwrap().acknowledge(generation);
        if accepted {
            wake.notify_all();
        }
        accepted
    }
}
impl Drop for FrameClock {
    fn drop(&mut self) {
        let (lock, wake) = &*self.state;
        lock.lock().unwrap().stopped = true;
        unsafe {
            let _ = SetEvent(self.changed.0);
        }
        wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_is_idle_until_needed_and_never_queues_multiple_frames() {
        let (send, receive) = std::sync::mpsc::channel();
        let clock = FrameClock::new(Arc::new(move |_, value| {
            let _ = send.send(value.parse::<u64>().unwrap());
        }))
        .unwrap();
        assert!(receive.recv_timeout(Duration::from_millis(25)).is_err());
        clock.set_active(true);
        let first = receive.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(receive.recv_timeout(Duration::from_millis(40)).is_err());
        clock.set_active(false);
        clock.set_active(true);
        let next = receive.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_ne!(first, next);
        assert!(!clock.acknowledge(first));
        assert!(clock.acknowledge(next));
        receive.recv_timeout(Duration::from_secs(2)).unwrap();
        clock.set_active(false);
        assert!(receive.recv_timeout(Duration::from_millis(40)).is_err());
        drop(clock);
    }
    #[test]
    fn stopped_or_superseded_frames_cannot_advance_a_new_animation() {
        let mut state = State::default();
        state.set_active(true);
        state.pending = true;
        let first = state.generation;
        state.set_active(false);
        assert!(!state.acknowledge(first));
        state.set_active(true);
        state.pending = true;
        assert!(!state.acknowledge(first));
        assert!(state.pending);
        assert!(state.acknowledge(state.generation));
        assert!(!state.acknowledge(state.generation));
    }
}
