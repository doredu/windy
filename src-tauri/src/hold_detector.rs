use std::time::{Duration, Instant};

pub struct HoldDetector {
    ctrl_down: bool,
    c_down: bool,
    hold_started_at: Option<Instant>,
    fired: bool,
}

impl HoldDetector {
    pub fn new() -> Self {
        Self { ctrl_down: false, c_down: false, hold_started_at: None, fired: false }
    }

    pub fn set_ctrl(&mut self, down: bool) {
        self.ctrl_down = down;
        self.on_state_change();
    }

    pub fn set_c(&mut self, down: bool) {
        self.c_down = down;
        self.on_state_change();
    }

    fn on_state_change(&mut self) {
        if !(self.ctrl_down && self.c_down) {
            self.hold_started_at = None;
            self.fired = false;
        }
    }

    /// Call periodically (e.g. every 100ms) from the watcher thread's timer.
    pub fn check(&mut self, now: Instant, threshold: Duration) -> bool {
        if self.ctrl_down && self.c_down {
            let start = *self.hold_started_at.get_or_insert(now);
            if !self.fired && now.duration_since(start) >= threshold {
                self.fired = true;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn does_not_fire_before_threshold() {
        let mut d = HoldDetector::new();
        let t0 = Instant::now();
        d.set_ctrl(true);
        d.set_c(true);
        assert!(!d.check(t0, Duration::from_secs(3)));
        assert!(!d.check(t0 + Duration::from_millis(2999), Duration::from_secs(3)));
    }

    #[test]
    fn fires_once_at_threshold_and_not_again_while_held() {
        let mut d = HoldDetector::new();
        let t0 = Instant::now();
        d.set_ctrl(true);
        d.set_c(true);
        d.check(t0, Duration::from_secs(3));
        assert!(d.check(t0 + Duration::from_secs(3), Duration::from_secs(3)));
        assert!(!d.check(t0 + Duration::from_secs(4), Duration::from_secs(3)));
    }

    #[test]
    fn releasing_c_resets_and_requires_full_hold_again() {
        let mut d = HoldDetector::new();
        let t0 = Instant::now();
        d.set_ctrl(true);
        d.set_c(true);
        d.check(t0, Duration::from_secs(3));
        d.set_c(false);
        assert!(!d.check(t0 + Duration::from_secs(3), Duration::from_secs(3)));
        d.set_c(true);
        assert!(!d.check(t0 + Duration::from_secs(3), Duration::from_secs(3)), "must restart the 3s window");
        assert!(d.check(t0 + Duration::from_secs(6), Duration::from_secs(3)));
    }

    #[test]
    fn only_ctrl_or_only_c_never_fires() {
        let mut d = HoldDetector::new();
        let t0 = Instant::now();
        d.set_ctrl(true);
        assert!(!d.check(t0 + Duration::from_secs(10), Duration::from_secs(3)));
    }
}
