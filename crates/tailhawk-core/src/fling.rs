//! Touch inertia — `UI-DESIGN.md` §12: the view keeps moving after the finger lifts.
//!
//! ## Why this is not in the shell
//!
//! A fling is four decisions, and all four are arithmetic: whether a flick counts as a fling at
//! all, how fast it starts, how fast it slows, and when it is spent. None of them needs a window,
//! so none of them lives near one. The shell's whole part is to feed samples in from
//! `WM_POINTERDOWN`/`WM_POINTERUPDATE`/`WM_POINTERUP` and to hand each tick's pixels to the
//! `Navigate::ByPixels` §6.4 already had.
//!
//! ## Why a trailing window, and not first-to-last
//!
//! A drag that crawls for a second and then flicks, and a flick that lands and then crawls, have
//! the same first and last sample. Averaging over the whole gesture would fling them identically,
//! and only one of them was a throw. So the release velocity is read from the samples inside
//! [`VELOCITY_WINDOW_MS`] of the lift, and a contact that spent that window sitting still does not
//! fling at all — which is also how a user stops a coast, by putting a finger down and taking it
//! off again.
//!
//! ## Why a half-life, and not a share per tick
//!
//! "Keep 85% each tick" hides the timer's period inside the feel: halve the period and the coast
//! decelerates twice as fast. A half-life in milliseconds does not care how often it is asked, and
//! [`Fling::step`] takes the elapsed time as an argument rather than assuming it was called on
//! time — a tick that arrives late during a re-shape covers the distance it missed instead of
//! losing it.

/// How far back from the lift the release velocity is read.
///
/// **The window has to be shorter than a flick, not longer.** A flick is a tenth of a second at the
/// outside; measured over 100 ms, a throw that follows a crawl has half a window of crawl in it and
/// reads at half speed. At 40 ms a digitiser sampling every 8–10 ms still gives four or five points
/// to fit, and none of them is from before the throw began.
pub const VELOCITY_WINDOW_MS: f32 = 40.0;

/// Below this, in pixels per second, a lift is a tap or the end of a slow drag and not a throw.
pub const FLING_MIN_VELOCITY: f32 = 120.0;

/// A lift this long after the last report means the finger had come to rest first, so whatever it
/// was doing before that is not a throw. Comfortably longer than any touch digitiser's report
/// interval — they run at 100 Hz and up — and comfortably shorter than a deliberate pause.
pub const STILL_MS: f32 = 60.0;

/// How long the coast takes to lose half its speed. Chosen by feel: a hard flick runs for roughly a
/// second and a half, and the tail is soft rather than a stop.
pub const HALF_LIFE_MS: f32 = 180.0;

/// Below this the coast is over — a pixel every few frames is not motion, it is a stuck view.
pub const SPENT_VELOCITY: f32 = 24.0;

/// A gesture's worth of motion: the samples while a contact is down, then the coast after it lifts.
///
/// One instance serves both phases, because they are the same quantity seen twice — the samples
/// exist to produce the velocity that the coast then spends. See the module note for why the
/// velocity is read from a trailing window and why the decay is a half-life.
#[derive(Clone, Debug, Default)]
pub struct Fling {
    /// `(time in ms, position in pixels)` while the contact is down, oldest first, pruned to the
    /// window. Positions are the *pointer's*, so a downward drag is an increasing position.
    samples: Vec<(f32, f32)>,
    /// Pixels per second the view is coasting at, positive meaning the content moves the way the
    /// finger was going. Zero when nothing is in flight.
    velocity: f32,
}

impl Fling {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a contact, discarding whatever came before — including a coast in progress.
    ///
    /// Putting a finger down on a moving view stops it. That is what every touch surface does, and
    /// it is the only way to stop a long coast without waiting it out.
    pub fn press(&mut self, now_ms: f32, position: f32) {
        self.samples.clear();
        self.velocity = 0.0;
        self.samples.push((now_ms, position));
    }

    /// A move while the contact is down. Returns the pixels travelled since the previous sample,
    /// which the caller scrolls by directly — a drag tracks the finger, it does not ease.
    pub fn drag(&mut self, now_ms: f32, position: f32) -> f32 {
        let moved = match self.samples.last() {
            Some(&(_, last)) => position - last,
            None => 0.0,
        };
        self.samples.push((now_ms, position));
        self.prune(now_ms);
        moved
    }

    /// Lift the contact and start a coast if the gesture earned one.
    ///
    /// **A lift long after the last report is a stop, whatever the samples say.** A digitiser stops
    /// reporting while a finger rests, so a pause before the lift is not in the samples at all — it
    /// is only visible as the gap between the last report and this call. Without that check a throw
    /// made a third of a second ago still flings, which is exactly the gesture a user makes to say
    /// *stop here*. The gap is tested rather than a repeat of the last position being appended,
    /// because a duplicate point prunes the real motion out of the window on a slow digitiser and
    /// turns every one of its gestures into a standstill.
    ///
    /// Returns whether anything is now in flight, so the caller knows whether to start its timer.
    pub fn release(&mut self, now_ms: f32) -> bool {
        let rested = self
            .samples
            .last()
            .is_some_and(|&(t, _)| now_ms - t > STILL_MS);
        self.prune(now_ms);
        self.velocity = if rested { 0.0 } else { self.release_velocity() };
        self.samples.clear();
        if self.velocity.abs() < FLING_MIN_VELOCITY {
            self.velocity = 0.0;
        }
        self.in_flight()
    }

    /// Stop dead. For anything that invalidates the motion — a cancelled contact, a document
    /// closing under it, or a remote session, where `UI-DESIGN.md` §11.5 turns inertia off because
    /// a coast defeats the scroll-region blit.
    pub fn halt(&mut self) {
        self.samples.clear();
        self.velocity = 0.0;
    }

    /// Whether a coast is still running.
    pub fn in_flight(&self) -> bool {
        self.velocity.abs() >= SPENT_VELOCITY
    }

    /// The current coasting speed in pixels per second, signed as the drag was.
    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    /// How many samples the window is currently holding. The estimate's resistance to one bad
    /// report is a function of this, so it is worth being able to assert on.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Advance the coast by `dt_ms` and report the pixels it covered.
    ///
    /// The distance is the integral of an exponential decay over the interval, not `v * dt`, so a
    /// long tick and two short ones covering the same span move the view the same distance. Once
    /// the speed falls under [`SPENT_VELOCITY`] the coast is over and the caller should stop
    /// asking.
    pub fn step(&mut self, dt_ms: f32) -> f32 {
        if !self.in_flight() || dt_ms <= 0.0 {
            return 0.0;
        }
        let decay = 0.5f32.powf(dt_ms / HALF_LIFE_MS);
        let after = self.velocity * decay;
        // ∫v₀·2^(−t/h) dt over [0, dt] — the exact area under the decay, in pixels.
        let travelled = (self.velocity - after) * (HALF_LIFE_MS / 1000.0) / std::f32::consts::LN_2;
        self.velocity = after;
        if !self.in_flight() {
            self.velocity = 0.0;
        }
        travelled
    }

    /// Pixels per second over the trailing window, or zero if it holds no span to measure.
    fn release_velocity(&self) -> f32 {
        let (&(t0, p0), &(t1, p1)) = match (self.samples.first(), self.samples.last()) {
            (Some(a), Some(b)) => (a, b),
            _ => return 0.0,
        };
        let span = t1 - t0;
        if span <= 0.0 {
            return 0.0;
        }
        (p1 - p0) / span * 1000.0
    }

    /// Drop samples older than the window, but never the one that gives it a span: two samples are
    /// the fewest that can express a speed.
    fn prune(&mut self, now_ms: f32) {
        while self.samples.len() > 2 && now_ms - self.samples[0].0 > VELOCITY_WINDOW_MS {
            self.samples.remove(0);
        }
    }
}

/// What the shell should do with one pointer message, decided before any of it is acted on.
///
/// The routing is as much of the feature as the physics and none of it needs a window: which pane
/// the contact is in, whether a second finger may steal a pan in progress, whether the point is on
/// the rows rather than the header or the status bar, and whether a capture change concerns the
/// contact that is actually panning. Every one of those is arithmetic over a handful of numbers,
/// and every one of them was a defect while it lived in the message loop.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TouchAction {
    /// Not ours. Let `DefWindowProc` have it, which promotes it to a mouse message — that is what
    /// keeps the menu bar, the command bar and a tap on a row working by finger.
    Ignore,
    /// Begin panning `pane`, with the contact at `y` in that pane's coordinates.
    Begin { pane: usize, y: f32 },
    /// Continue the pan; `y` is in the panning pane's coordinates.
    Move { y: f32 },
    /// Lift: end the pan and start a coast if the gesture earned one.
    End,
    /// Abandon the pan and any coast without starting a new one.
    Cancel,
}

/// Which pointer message this is. The shell maps `WM_POINTER*` onto it and decides nothing itself.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Update,
    Up,
    CaptureLost,
}

/// The window's geometry as the decision needs it: where each pane starts, and the insets inside a
/// pane that are chrome rather than rows.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Panes<'a> {
    /// The top of each pane in client coordinates, in order.
    ///
    /// **The caller passes the one pane the contact is in**, not all of them. This module reasons
    /// in one dimension, which is right for the gesture — but panes may sit side by side, where
    /// every top is zero and this list alone cannot tell them apart. Handed all of them, it read
    /// every touch as landing in the last pane.
    pub tops: &'a [f32],
    /// Menu bar, command bar and column header — everything above the first row in a pane.
    pub top_inset: f32,
    /// Status bar and detail pane — everything below the last row in a pane.
    pub bottom_inset: f32,
    /// The bottom of the last pane in client coordinates.
    pub bottom: f32,
}

/// Decide what one pointer message means.
///
/// `panning` is the id of the contact already panning, if any. A second contact is refused rather
/// than allowed to take over: two fingers on a log are a pinch or a two-finger scroll, and §12 asks
/// for neither — but taking the second one *silently* is worse than refusing it, because the first
/// finger's own updates then fall through to `DefWindowProc` and drag a selection across a view the
/// second finger is scrolling.
///
/// `covered` is whether a modal surface is over the rows; a pan under one would scroll text the box
/// is hiding.
pub fn decide(
    phase: TouchPhase,
    id: u32,
    y: f32,
    panning: Option<u32>,
    covered: bool,
    panes: Panes<'_>,
) -> TouchAction {
    match phase {
        // A capture change for some other contact says nothing about ours. Cancelling on it would
        // let a tap on the menu bar with a second finger kill a coast started by the first.
        TouchPhase::CaptureLost => match panning {
            Some(p) if p == id => TouchAction::Cancel,
            _ => TouchAction::Ignore,
        },
        TouchPhase::Down => {
            if covered || panning.is_some() {
                return TouchAction::Ignore;
            }
            match pane_of(y, panes) {
                Some((pane, local)) => TouchAction::Begin { pane, y: local },
                None => TouchAction::Ignore,
            }
        }
        TouchPhase::Update if panning == Some(id) => {
            let top = panning_pane_top(y, panes);
            TouchAction::Move { y: y - top }
        }
        TouchPhase::Up if panning == Some(id) => TouchAction::End,
        _ => TouchAction::Ignore,
    }
}

/// The pane containing `y` and the offset within it, if `y` is on that pane's **rows** — not its
/// header band, where the column dividers are dragged, and not the status bar or detail pane below
/// it, which are chrome and must keep behaving like chrome.
fn pane_of(y: f32, panes: Panes<'_>) -> Option<(usize, f32)> {
    let i = panes.tops.iter().rposition(|&t| y >= t)?;
    let top = panes.tops[i];
    let next = panes.tops.get(i + 1).copied().unwrap_or(panes.bottom);
    let local = y - top;
    (local >= panes.top_inset && y < next - panes.bottom_inset).then_some((i, local))
}

/// The top of whichever pane `y` falls in, for translating a move already known to belong to the
/// pan in progress. A drag that leaves its pane keeps scrolling the pane it began in, so this is a
/// coordinate translation and not a re-routing.
fn panning_pane_top(y: f32, panes: Panes<'_>) -> f32 {
    panes
        .tops
        .iter()
        .rposition(|&t| y >= t)
        .map_or(0.0, |i| panes.tops[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &[f32] = &[0.0];
    const SPLIT: &[f32] = &[0.0, 500.0];

    fn one_pane() -> Panes<'static> {
        Panes {
            tops: ONE,
            top_inset: 100.0,
            bottom_inset: 30.0,
            bottom: 1000.0,
        }
    }

    fn split() -> Panes<'static> {
        Panes {
            tops: SPLIT,
            top_inset: 100.0,
            bottom_inset: 30.0,
            bottom: 1000.0,
        }
    }

    #[test]
    fn a_contact_on_the_rows_begins_a_pan() {
        assert_eq!(
            decide(TouchPhase::Down, 7, 400.0, None, false, one_pane()),
            TouchAction::Begin { pane: 0, y: 400.0 }
        );
    }

    #[test]
    fn the_chrome_is_left_to_the_mouse() {
        for y in [0.0, 50.0, 99.0] {
            assert_eq!(
                decide(TouchPhase::Down, 7, y, None, false, one_pane()),
                TouchAction::Ignore,
                "y={y} is the menu bar, command bar or header — a finger there is a click"
            );
        }
    }

    #[test]
    fn the_status_bar_below_the_rows_is_chrome_too() {
        for y in [975.0, 990.0, 999.0] {
            assert_eq!(
                decide(TouchPhase::Down, 7, y, None, false, one_pane()),
                TouchAction::Ignore,
                "y={y} is inside the bottom inset and must not start a pan"
            );
        }
    }

    #[test]
    fn a_modal_box_over_the_rows_refuses_the_pan() {
        assert_eq!(
            decide(TouchPhase::Down, 7, 400.0, None, true, one_pane()),
            TouchAction::Ignore
        );
    }

    #[test]
    fn a_second_finger_does_not_steal_a_pan() {
        assert_eq!(
            decide(TouchPhase::Down, 9, 400.0, Some(7), false, one_pane()),
            TouchAction::Ignore,
            "the first finger is still panning; the second is not a new gesture"
        );
    }

    #[test]
    fn a_move_from_a_contact_that_is_not_panning_is_not_ours() {
        assert_eq!(
            decide(TouchPhase::Update, 9, 400.0, Some(7), false, one_pane()),
            TouchAction::Ignore
        );
        assert_eq!(
            decide(TouchPhase::Update, 9, 400.0, None, false, one_pane()),
            TouchAction::Ignore,
            "and an update with no pan at all is a stray, not a pan"
        );
    }

    #[test]
    fn a_lift_without_a_matching_press_is_not_ours() {
        assert_eq!(
            decide(TouchPhase::Up, 9, 400.0, None, false, one_pane()),
            TouchAction::Ignore
        );
        assert_eq!(
            decide(TouchPhase::Up, 9, 400.0, Some(7), false, one_pane()),
            TouchAction::Ignore
        );
    }

    #[test]
    fn a_capture_change_only_concerns_the_contact_that_is_panning() {
        assert_eq!(
            decide(TouchPhase::CaptureLost, 7, 0.0, Some(7), false, one_pane()),
            TouchAction::Cancel
        );
        assert_eq!(
            decide(TouchPhase::CaptureLost, 9, 0.0, Some(7), false, one_pane()),
            TouchAction::Ignore,
            "another contact losing capture must not kill this pan"
        );
        assert_eq!(
            decide(TouchPhase::CaptureLost, 9, 0.0, None, false, one_pane()),
            TouchAction::Ignore,
            "and a capture change during a coast is not the coast's business"
        );
    }

    #[test]
    fn a_contact_in_the_lower_pane_pans_the_lower_pane() {
        assert_eq!(
            decide(TouchPhase::Down, 7, 700.0, None, false, split()),
            TouchAction::Begin { pane: 1, y: 200.0 },
            "the split's lower pane starts at 500, so 700 is 200 into it"
        );
    }

    #[test]
    fn the_lower_panes_own_header_band_is_not_rows() {
        for y in [500.0, 560.0, 599.0] {
            assert_eq!(
                decide(TouchPhase::Down, 7, y, None, false, split()),
                TouchAction::Ignore,
                "y={y} is the lower pane's chrome, not its rows"
            );
        }
    }

    #[test]
    fn the_upper_panes_bottom_edge_is_not_rows_either() {
        for y in [475.0, 499.0] {
            assert_eq!(
                decide(TouchPhase::Down, 7, y, None, false, split()),
                TouchAction::Ignore,
                "y={y} is within the upper pane's bottom inset"
            );
        }
    }

    #[test]
    fn a_move_is_reported_in_its_own_panes_coordinates() {
        assert_eq!(
            decide(TouchPhase::Update, 7, 720.0, Some(7), false, split()),
            TouchAction::Move { y: 220.0 }
        );
    }

    /// Run a coast to a standstill, returning the total pixels and the number of ticks.
    fn coast(f: &mut Fling, dt_ms: f32) -> (f32, u32) {
        let (mut total, mut ticks) = (0.0, 0);
        while f.in_flight() && ticks < 10_000 {
            total += f.step(dt_ms);
            ticks += 1;
        }
        (total, ticks)
    }

    #[test]
    fn a_flick_coasts_on_after_the_finger_lifts() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        // 600 px in 60 ms — 10 000 px/s, a hard throw.
        for i in 1..=6 {
            f.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        assert!(f.release(60.0), "a hard flick is a fling");
        let (travelled, ticks) = coast(&mut f, 16.0);
        assert!(
            travelled > 200.0,
            "a throw that fast should carry the view a long way, went {travelled}"
        );
        assert!(
            ticks > 10,
            "and should take more than a few frames to do it"
        );
    }

    #[test]
    fn a_slow_drag_does_not_fling() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        // 40 px over 400 ms — 100 px/s, under the threshold and plainly a drag.
        for i in 1..=8 {
            f.drag(i as f32 * 50.0, i as f32 * 5.0);
        }
        assert!(!f.release(400.0), "a slow drag ends where it is let go");
        assert_eq!(f.step(16.0), 0.0);
    }

    /// The digitiser stops reporting while the finger rests, which is what real hardware does when
    /// nothing changes — so the rest is visible only as the gap before the lift. Without the lift
    /// itself in the samples that gap does not exist, and a throw made a third of a second ago
    /// still flings.
    #[test]
    fn a_rest_before_the_lift_kills_the_throw_even_with_no_reports_during_it() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        f.drag(10.0, 100.0);
        f.drag(20.0, 200.0);
        assert!(
            !f.release(320.0),
            "300 ms passed between the last report and the lift — the finger had stopped"
        );
    }

    /// A digitiser that repeats its last position once before the lift must not turn a hard throw
    /// into no throw at all.
    #[test]
    fn one_repeated_report_does_not_swallow_a_flick() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        for i in 1..=4 {
            f.drag(i as f32 * 12.0, i as f32 * 120.0);
        }
        f.drag(60.0, 480.0);
        assert!(
            f.release(64.0),
            "a stuck final report is jitter, not a decision to stop"
        );
        assert!(f.velocity() > 5_000.0, "and it is still a hard throw");
    }

    /// A digitiser reporting more slowly than the window is wide would otherwise be pruned down to
    /// a single point, and one point has no speed — every gesture on such a device would read as
    /// nothing at all. The floor in `prune` is what stops that, and it is the only thing that does.
    #[test]
    fn reports_slower_than_the_window_still_leave_a_span_to_measure() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        f.drag(45.0, 450.0);
        f.drag(90.0, 900.0);
        assert!(
            f.sample_count() >= 2,
            "pruned to {} — a speed needs two points",
            f.sample_count()
        );
        assert!(
            f.release(94.0),
            "10 000 px/s is a throw however sparsely it was sampled"
        );
    }

    #[test]
    fn the_window_keeps_enough_samples_to_survive_jitter() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        for i in 1..=6 {
            f.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        assert!(
            f.sample_count() >= 3,
            "two points is a secant and one bad report ruins it; got {}",
            f.sample_count()
        );
    }

    /// The two thresholds are different numbers and the relationship between them is load-bearing:
    /// if a release could arm a coast that `step` refuses to spend, the timer would run to no
    /// effect for ever.
    #[test]
    fn a_release_never_arms_a_coast_that_step_will_not_spend() {
        const { assert!(FLING_MIN_VELOCITY > SPENT_VELOCITY) };
        for speed in [0.0, 10.0, 23.0, 24.0, 100.0, 119.0, 120.0, 500.0, 9_000.0] {
            let mut f = Fling::new();
            f.press(0.0, 0.0);
            f.drag(100.0, speed / 10.0);
            let armed = f.release(100.0);
            assert_eq!(
                armed,
                f.velocity() != 0.0,
                "release said {armed} but the velocity is {}",
                f.velocity()
            );
            assert_eq!(armed, f.in_flight(), "and in_flight must agree with both");
        }
    }

    #[test]
    fn a_finger_that_stops_before_lifting_kills_the_throw() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        for i in 1..=6 {
            f.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        // Then it rests for longer than the window before coming off. The samples inside the
        // window are all at 600, so there is no speed to release.
        for i in 1..=6 {
            f.drag(60.0 + i as f32 * 30.0, 600.0);
        }
        assert!(
            !f.release(240.0),
            "landing the flick and holding still is how a user says 'no, stop here'"
        );
    }

    #[test]
    fn the_crawl_before_a_flick_is_not_part_of_the_flick() {
        let mut slow_then_fast = Fling::new();
        slow_then_fast.press(0.0, 0.0);
        for i in 1..=10 {
            slow_then_fast.drag(i as f32 * 50.0, i as f32 * 2.0);
        }
        for i in 1..=5 {
            slow_then_fast.drag(500.0 + i as f32 * 10.0, 20.0 + i as f32 * 100.0);
        }
        slow_then_fast.release(550.0);

        let mut just_fast = Fling::new();
        just_fast.press(0.0, 0.0);
        for i in 1..=5 {
            just_fast.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        just_fast.release(50.0);

        let (a, b) = (slow_then_fast.velocity(), just_fast.velocity());
        assert!(
            (a - b).abs() / b.abs() < 0.05,
            "the same flick preceded by a crawl should throw the same: {a} vs {b}"
        );
    }

    #[test]
    fn a_late_tick_covers_the_distance_it_missed() {
        let mut one_long = Fling::new();
        one_long.press(0.0, 0.0);
        for i in 1..=6 {
            one_long.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        one_long.release(60.0);

        let mut many_short = one_long.clone();

        let long = one_long.step(64.0);
        let short: f32 = (0..4).map(|_| many_short.step(16.0)).sum();
        assert!(
            (long - short).abs() < 0.5,
            "one 64 ms tick and four 16 ms ticks must cover the same ground: {long} vs {short}"
        );
        assert!(
            (one_long.velocity() - many_short.velocity()).abs() < 0.5,
            "and leave the coast at the same speed"
        );
    }

    #[test]
    fn the_half_life_does_what_it_says() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        for i in 1..=6 {
            f.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        f.release(60.0);
        let before = f.velocity();
        f.step(HALF_LIFE_MS);
        assert!(
            (f.velocity() / before - 0.5).abs() < 0.01,
            "one half-life should halve the speed, got {} of {before}",
            f.velocity()
        );
    }

    #[test]
    fn a_coast_stops_rather_than_creeping_forever() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        for i in 1..=6 {
            f.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        f.release(60.0);
        let (_, ticks) = coast(&mut f, 16.0);
        assert!(
            ticks < 300,
            "an exponential tail still has to end; took {ticks} frames"
        );
        assert!(!f.in_flight());
        assert_eq!(f.velocity(), 0.0, "and it ends at rest, not at epsilon");
    }

    #[test]
    fn a_finger_down_stops_a_coast() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        for i in 1..=6 {
            f.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        f.release(60.0);
        assert!(f.in_flight());
        f.press(100.0, 0.0);
        assert!(!f.in_flight(), "a finger on a moving view stops it");
        assert_eq!(f.step(16.0), 0.0);
    }

    #[test]
    fn halt_stops_it_too_for_the_remote_session_case() {
        let mut f = Fling::new();
        f.press(0.0, 0.0);
        for i in 1..=6 {
            f.drag(i as f32 * 10.0, i as f32 * 100.0);
        }
        f.release(60.0);
        f.halt();
        assert!(!f.in_flight());
    }

    #[test]
    fn a_drag_tracks_the_finger_exactly() {
        let mut f = Fling::new();
        f.press(0.0, 100.0);
        assert_eq!(f.drag(10.0, 130.0), 30.0);
        assert_eq!(f.drag(20.0, 125.0), -5.0, "and follows it back up");
        assert_eq!(f.drag(30.0, 125.0), 0.0);
    }

    #[test]
    fn direction_is_kept_both_ways() {
        let mut up = Fling::new();
        up.press(0.0, 0.0);
        for i in 1..=6 {
            up.drag(i as f32 * 10.0, i as f32 * -100.0);
        }
        up.release(60.0);
        assert!(up.velocity() < 0.0);
        assert!(up.step(16.0) < 0.0, "a coast goes the way it was thrown");
    }

    #[test]
    fn a_tap_is_not_a_gesture() {
        let mut f = Fling::new();
        f.press(0.0, 400.0);
        assert!(!f.release(20.0), "down and straight up moves nothing");
        assert_eq!(f.velocity(), 0.0);
    }
}
