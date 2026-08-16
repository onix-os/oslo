//! The bookkeeping, without a Lua interpreter to fire into. What a timer *does* when it fires is
//! covered by `tests/lua_corpus/timers.lua`, which runs through the real binary.

use super::*;

/// Every test shares one thread-local list, so they cannot run beside each other.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    TIMERS.with(|timers| timers.borrow_mut().clear());
    guard
}

/// A function value to hang a timer on. Never called here.
fn nothing() -> Value {
    super::super::util::native("nothing", |_, _| Ok(Vec::new()))
}

#[test]
fn a_delay_and_a_function_are_both_required() {
    let _alone = alone();
    assert!(schedule(&[], "oslo.after", false).is_err());
    assert!(schedule(&[Value::int(10)], "oslo.after", false).is_err());
    assert!(schedule(&[nothing(), Value::int(10)], "oslo.after", false).is_err());
    assert!(schedule(&[Value::int(10), nothing()], "oslo.after", false).is_ok());
}

#[test]
fn a_delay_that_is_not_a_delay_is_refused() {
    let _alone = alone();
    for silly in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(
            schedule(&[Value::float(silly), nothing()], "oslo.after", false).is_err(),
            "{silly} was accepted"
        );
    }
}

/// **A repeating timer of zero would be due forever**, and every check would fire it again.
#[test]
fn a_repeat_of_zero_is_widened_to_something_that_can_pass() {
    let _alone = alone();
    schedule(&[Value::int(0), nothing()], "oslo.every", true).expect("scheduled");
    let every = TIMERS.with(|timers| timers.borrow()[0].every);
    assert_eq!(every, Some(Duration::from_millis(1)));
}

#[test]
fn a_handle_stops_its_own_timer_and_says_whether_it_had_one() {
    let _alone = alone();
    let answered = schedule(&[Value::int(60_000), nothing()], "oslo.after", false).expect("ok");
    let Some(Value::Table(handle)) = answered.first() else {
        panic!("no handle")
    };
    let Value::Function(stop) = handle.borrow().get_str("stop") else {
        panic!("no stop")
    };
    assert!(any(), "the timer should be waiting");

    let call = |f: &std::rc::Rc<oslo_base::value::Function>| match &**f {
        oslo_base::value::Function::Native { call, .. } => {
            let interp = oslo_lua::Interp::new("test");
            call(&interp, Vec::new()).expect("stop")
        }
        _ => panic!("not native"),
    };
    assert_eq!(call(&stop).first().map(Value::truthy), Some(true));
    assert!(!any(), "stopping should take it out of the list");
    // Stopping twice says there was nothing left to stop.
    assert_eq!(call(&stop).first().map(Value::truthy), Some(false));
}

#[test]
fn a_timer_that_is_not_due_is_left_alone() {
    let _alone = alone();
    schedule(&[Value::int(60_000), nothing()], "oslo.after", false).expect("ok");
    assert!(settle(Instant::now()).is_empty(), "not due for a minute");
    assert!(any(), "and still waiting");
}

/// A one-shot goes out of the list when it comes due; a repeat stays and is due again.
#[test]
fn settling_drops_a_one_shot_and_reschedules_a_repeat() {
    let _alone = alone();
    schedule(&[Value::int(0), nothing()], "oslo.after", false).expect("ok");
    schedule(&[Value::int(1), nothing()], "oslo.every", true).expect("ok");
    std::thread::sleep(Duration::from_millis(5));

    let now = Instant::now();
    assert_eq!(settle(now).len(), 2, "both were due");
    let left: Vec<Option<Duration>> =
        TIMERS.with(|timers| timers.borrow().iter().map(|timer| timer.every).collect());
    assert_eq!(left.len(), 1, "the one-shot should be gone");
    assert_eq!(left[0], Some(Duration::from_millis(1)));
    assert!(
        TIMERS.with(|timers| timers.borrow()[0].due > now),
        "a fired repeat is due again later, not still due now"
    );
}

/// **A repeat does not catch up.** A shell can sit at a prompt for an hour; sixty missed ticks
/// arriving at once is never what was wanted.
#[test]
fn a_long_wait_produces_one_tick_rather_than_all_the_missed_ones() {
    let _alone = alone();
    schedule(&[Value::int(1), nothing()], "oslo.every", true).expect("ok");
    let much_later = Instant::now() + Duration::from_secs(3600);
    assert_eq!(settle(much_later).len(), 1, "one tick, not 3.6 million");
    // Asked again at the *same* instant it answers nothing: it was rescheduled from there, so the
    // next tick is a millisecond after this one rather than immediately.
    assert!(settle(much_later).is_empty());
    assert_eq!(
        settle(much_later + Duration::from_millis(2)).len(),
        1,
        "and one tick when the next one is genuinely due"
    );
}
