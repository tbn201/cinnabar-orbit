pub const REQUEST: &str = "orbit.tpaura";
pub const READY: &str = "orbit.tpaura-ready";
pub const CAPABILITY: u32 = 524288;
pub const MAX_RANGE: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub players: bool,
    pub mobs: bool,
    pub range: f32,
    pub rate: f32,
}

impl Policy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, players, mobs, range, rate] = values else {
            return None;
        };
        (*version == 1.0
            && [*players, *mobs].iter().all(|v| *v == 0.0 || *v == 1.0)
            && (*players == 1.0 || *mobs == 1.0)
            && range.is_finite()
            && (2.0..=MAX_RANGE).contains(range)
            && rate.is_finite()
            && (1.0..=5.0).contains(rate)
            && rate.fract() == 0.0)
            .then_some(Self {
                players: *players == 1.0,
                mobs: *mobs == 1.0,
                range: *range,
                rate: *rate,
            })
    }

    pub fn values(self) -> [f32; 5] {
        [
            1.0,
            u8::from(self.players) as f32,
            u8::from(self.mobs) as f32,
            self.range,
            self.rate,
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub scope: (u64, i32, u64),
    pub now: f64,
    pub feet: [f32; 3],
    pub held: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trip {
    pub target: u64,
    pub home: [f32; 3],
    pub landing: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Idle,
    Move([f32; 3]),
    Aim(u64),
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Outbound(Trip, f64),
    Aiming(Trip, f64),
    Returning(Trip, f64),
}

#[derive(Default)]
pub struct Aura {
    scope: Option<(u64, i32, u64)>,
    phase: Phase,
    last: Option<f64>,
    next: f64,
    interval: f64,
}

impl Aura {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn tick(
        &mut self,
        frame: Frame,
        policy: Option<Policy>,
        trip: impl FnOnce() -> Option<Trip>,
    ) -> Action {
        if !frame.now.is_finite() || !frame.feet.iter().all(|v| v.is_finite()) {
            self.reset();
            return Action::Idle;
        }
        if self.scope != Some(frame.scope)
            || self
                .last
                .is_some_and(|last| frame.now < last || frame.now - last > 0.5)
        {
            self.reset();
            self.scope = Some(frame.scope);
            self.next = frame.now + 0.25;
        }
        self.last = Some(frame.now);
        match self.phase {
            Phase::Idle => {
                let Some(policy) = policy.filter(|_| frame.held && frame.now >= self.next) else {
                    return Action::Idle;
                };
                self.next = frame.now + 0.1;
                let Some(trip) = trip().filter(|trip| {
                    trip.target != 0
                        && close(trip.home, frame.feet, 0.05)
                        && trip.landing.iter().all(|v| v.is_finite())
                        && close(trip.home, trip.landing, policy.range + 0.01)
                }) else {
                    return Action::Idle;
                };
                self.phase = Phase::Outbound(trip, frame.now + 0.25);
                self.interval = 1.0 / f64::from(policy.rate);
                self.next = frame.now + self.interval;
                Action::Move(trip.landing)
            }
            Phase::Outbound(trip, until) => {
                if close(frame.feet, trip.landing, 0.2) {
                    self.phase = Phase::Aiming(trip, frame.now + 0.75);
                    if policy.is_some() && frame.held {
                        Action::Aim(trip.target)
                    } else {
                        self.return_home(trip, frame.now)
                    }
                } else if policy.is_none()
                    || !frame.held
                    || frame.now > until
                    || !close(frame.feet, trip.home, 0.2)
                {
                    self.phase = Phase::Idle;
                    self.next = frame.now + 1.0;
                    Action::Idle
                } else {
                    Action::Move(trip.landing)
                }
            }
            Phase::Aiming(trip, until) => {
                if !close(frame.feet, trip.landing, 0.35) {
                    self.phase = Phase::Idle;
                    self.next = frame.now + 1.0;
                    Action::Idle
                } else if frame.now > until || policy.is_none() || !frame.held {
                    self.return_home(trip, frame.now)
                } else {
                    Action::Aim(trip.target)
                }
            }
            Phase::Returning(trip, until) => {
                if close(frame.feet, trip.home, 0.2) {
                    self.phase = Phase::Idle;
                    self.next = self.next.max(frame.now + 0.1);
                    Action::Idle
                } else if frame.now > until || !close(frame.feet, trip.landing, 0.35) {
                    self.phase = Phase::Idle;
                    self.next = frame.now + 1.0;
                    Action::Idle
                } else {
                    Action::Move(trip.home)
                }
            }
        }
    }

    pub fn attacked(&mut self, now: f64) {
        if let Phase::Aiming(trip, _) = self.phase {
            self.next = self.next.max(now + self.interval);
            self.phase = Phase::Returning(trip, now + 0.25);
        }
    }

    pub fn abandon_target(&mut self, now: f64) {
        self.attacked(now);
    }

    fn return_home(&mut self, trip: Trip, now: f64) -> Action {
        self.phase = Phase::Returning(trip, now + 0.25);
        Action::Move(trip.home)
    }
}

pub fn close(a: [f32; 3], b: [f32; 3], radius: f32) -> bool {
    a.iter().chain(b.iter()).all(|v| v.is_finite())
        && (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>() <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy {
            players: true,
            mobs: false,
            range: 6.0,
            rate: 3.0,
        }
    }
    fn trip() -> Option<Trip> {
        Some(Trip {
            target: 9,
            home: [0.0; 3],
            landing: [4.0, 0.0, 0.0],
        })
    }
    fn frame(now: f64, x: f32) -> Frame {
        Frame {
            scope: (1, 0, 1),
            now,
            feet: [x, 0.0, 0.0],
            held: true,
        }
    }
    fn started(aura: &mut Aura) {
        assert_eq!(
            aura.tick(frame(0.0, 0.0), Some(policy()), trip),
            Action::Idle
        );
        assert_eq!(
            aura.tick(frame(0.3, 0.0), Some(policy()), trip),
            Action::Move([4.0, 0.0, 0.0])
        );
    }
    #[test]
    fn policy_rejects_bad_versions_flags_ranges_and_rates() {
        assert_eq!(Policy::parse(&policy().values()), Some(policy()));
        for values in [
            vec![],
            vec![2.0, 1.0, 0.0, 6.0, 3.0],
            vec![1.0, 2.0, 0.0, 6.0, 3.0],
            vec![1.0, 0.0, 0.0, 6.0, 3.0],
            vec![1.0, 1.0, 0.0, 9.0, 3.0],
            vec![1.0, 1.0, 0.0, f32::NAN, 3.0],
            vec![1.0, 1.0, 0.0, 6.0, 3.5],
        ] {
            assert_eq!(Policy::parse(&values), None);
        }
    }
    #[test]
    fn aims_only_after_arrival_and_returns_after_one_attack() {
        let mut aura = Aura::default();
        started(&mut aura);
        assert_eq!(
            aura.tick(frame(0.32, 0.0), Some(policy()), trip),
            Action::Move([4.0, 0.0, 0.0])
        );
        assert_eq!(
            aura.tick(frame(0.35, 4.0), Some(policy()), trip),
            Action::Aim(9)
        );
        aura.attacked(0.36);
        assert_eq!(
            aura.tick(frame(0.37, 4.0), Some(policy()), trip),
            Action::Move([0.0; 3])
        );
        assert_eq!(
            aura.tick(frame(0.4, 0.0), Some(policy()), trip),
            Action::Idle
        );
    }
    #[test]
    fn corrections_dimension_changes_and_lag_discard_the_old_home() {
        for scope in [(2, 0, 1), (1, 1, 1), (1, 0, 2)] {
            let mut aura = Aura::default();
            started(&mut aura);
            assert_eq!(
                aura.tick(
                    Frame {
                        scope,
                        ..frame(0.35, 4.0)
                    },
                    Some(policy()),
                    trip
                ),
                Action::Idle
            );
        }
        let mut aura = Aura::default();
        started(&mut aura);
        assert_eq!(
            aura.tick(frame(2.0, 4.0), Some(policy()), trip),
            Action::Idle
        );
    }
    #[test]
    fn release_and_disable_return_only_after_confirmed_arrival() {
        let mut aura = Aura::default();
        started(&mut aura);
        assert_eq!(
            aura.tick(
                Frame {
                    held: false,
                    ..frame(0.35, 4.0)
                },
                None,
                trip
            ),
            Action::Move([0.0; 3])
        );
        let mut aura = Aura::default();
        started(&mut aura);
        assert_eq!(aura.tick(frame(0.7, 0.0), None, trip), Action::Idle);
    }
    #[test]
    fn an_empty_target_scan_is_throttled() {
        let mut aura = Aura::default();
        let scans = std::cell::Cell::new(0);
        let scan = || {
            scans.set(scans.get() + 1);
            None
        };
        for now in [0.0, 0.3, 0.31, 0.32, 0.39, 0.41] {
            assert_eq!(
                aura.tick(frame(now, 0.0), Some(policy()), scan),
                Action::Idle
            );
        }
        assert_eq!(scans.get(), 2);
    }

    #[test]
    fn release_before_the_fixed_tick_cancels_the_outbound_move() {
        let mut aura = Aura::default();
        started(&mut aura);
        assert_eq!(
            aura.tick(
                Frame {
                    held: false,
                    ..frame(0.32, 0.0)
                },
                None,
                trip
            ),
            Action::Idle
        );
        assert_eq!(aura.tick(frame(0.34, 0.0), None, trip), Action::Idle);
    }

    #[test]
    fn attack_rate_is_measured_from_the_last_attack_not_the_departure() {
        let mut aura = Aura::default();
        started(&mut aura);
        assert_eq!(
            aura.tick(frame(0.35, 4.0), Some(policy()), trip),
            Action::Aim(9)
        );
        aura.attacked(0.6);
        assert_eq!(
            aura.tick(frame(0.62, 0.0), Some(policy()), trip),
            Action::Idle
        );
        assert_eq!(
            aura.tick(frame(0.8, 0.0), Some(policy()), trip),
            Action::Idle
        );
        assert_eq!(
            aura.tick(frame(0.95, 0.0), Some(policy()), trip),
            Action::Move([4.0, 0.0, 0.0])
        );
    }

    #[test]
    fn blocked_or_unexpected_movement_never_produces_an_attack() {
        let mut aura = Aura::default();
        started(&mut aura);
        assert_eq!(
            aura.tick(frame(0.35, 2.0), Some(policy()), trip),
            Action::Idle
        );
        let mut aura = Aura::default();
        started(&mut aura);
        assert_eq!(
            aura.tick(frame(0.7, 0.0), Some(policy()), trip),
            Action::Idle
        );
    }
}
