use super::*;

fn guard() -> LoopGuard {
    LoopGuard::new(LoopGuardConfig::default())
}

#[test]
fn identical_calls_warn_first_then_block() {
    let mut g = guard();
    for _ in 0..3 {
        assert_eq!(g.check_call("открой", "открой блокнот"), Verdict::default());
    }
    // the 4th identical call: a warning only (warn before block), it still goes through
    let v = g.check_call("открой", "открой блокнот");
    assert!(v.warned && !v.blocked, "{:?}", v);
    // the 5th: blocked
    let v = g.check_call("открой", "открой блокнот");
    assert!(v.blocked && !v.warned && v.reason.contains("Identical"), "{:?}", v);
}

#[test]
fn different_calls_do_not_trip_the_identical_rule() {
    let mut g = guard();
    for i in 0..5 {
        let v = g.check_call(&format!("tool{}", i), "x");
        assert_eq!(v, Verdict::default());
    }
}

#[test]
fn a_tool_over_its_budget_is_blocked() {
    let mut g = LoopGuard::new(LoopGuardConfig { warn_before_block: false, max_identical_calls: 100, ..Default::default() });
    for i in 0..5 {
        assert!(!g.check_call("громкость", &format!("v{}", i)).blocked);
    }
    let v = g.check_call("громкость", "v99");
    assert!(v.blocked && v.reason.contains("budget"), "{:?}", v);
}

#[test]
fn ping_pong_is_detected() {
    let mut g = LoopGuard::new(LoopGuardConfig { warn_before_block: false, max_identical_calls: 100, poll_tool_budget: 100, ..Default::default() });
    let mut blocked = false;
    for i in 0..8 {
        let tool = if i % 2 == 0 { "A" } else { "B" };
        let v = g.check_call(tool, &format!("arg{}", i));
        if v.blocked {
            blocked = true;
            assert!(v.reason.contains("ping-pong"));
            break;
        }
    }
    assert!(blocked, "A-B-A-B-A-B must be caught");
    // a period-3 cycle as well
    let mut g = LoopGuard::new(LoopGuardConfig { warn_before_block: false, max_identical_calls: 100, poll_tool_budget: 100, ..Default::default() });
    let seq = ["A", "B", "C", "A", "B", "C"];
    let last = seq.iter().enumerate().map(|(i, t)| g.check_call(t, &format!("x{}", i))).last().unwrap();
    assert!(last.blocked, "{:?}", last);
}

#[test]
fn varied_work_is_not_a_loop() {
    let mut g = guard();
    for (i, t) in ["открой", "громкость", "таймер", "яркость", "найди", "пауза"].iter().enumerate() {
        assert_eq!(g.check_call(t, &format!("a{}", i)), Verdict::default());
    }
}

#[test]
fn reset_and_disabled() {
    let mut g = guard();
    for _ in 0..3 {
        g.check_call("t", "a");
    }
    g.reset();
    assert_eq!(g.check_call("t", "a"), Verdict::default());
    let mut off = LoopGuard::new(LoopGuardConfig { enabled: false, ..Default::default() });
    for _ in 0..50 {
        assert_eq!(off.check_call("t", "a"), Verdict::default());
    }
}
