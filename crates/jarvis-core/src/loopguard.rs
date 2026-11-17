// Protection against degenerate loops of actions started by the AI.
//
// A port of the idea and the algorithm of `LoopGuard` from OpenJarvis (https://github.com/open-jarvis/OpenJarvis,
// src/openjarvis/agents/loop_guard.py, Apache-2.0), rewritten in Rust for the Jarvis actions:
//   1. the same call (tool + arguments) more than `max_identical_calls` times is blocked;
//   2. one tool called more than `poll_tool_budget` times is blocked;
//   3. a repeating pattern of tools (A-B-A-B, A-B-C-A-B-C) is blocked;
//   4. "warn before block": the first time a cycle is seen it is only reported, the second time it is blocked.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug)]
pub struct LoopGuardConfig {
    pub enabled: bool,
    pub max_identical_calls: usize,
    pub ping_pong_window: usize,
    pub poll_tool_budget: usize,
    pub warn_before_block: bool,
}

impl Default for LoopGuardConfig {
    fn default() -> Self {
        LoopGuardConfig { enabled: true, max_identical_calls: 3, ping_pong_window: 6, poll_tool_budget: 5, warn_before_block: true }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    pub blocked: bool,
    pub warned: bool,
    pub reason: String,
}

pub struct LoopGuard {
    cfg: LoopGuardConfig,
    call_counts: HashMap<u64, usize>,
    tool_sequence: VecDeque<String>,
    per_tool: HashMap<String, usize>,
    warned: HashSet<String>,
}

fn hash_call(tool: &str, args: &str) -> u64 {
    let mut h = DefaultHasher::new();
    tool.hash(&mut h);
    ":".hash(&mut h);
    args.hash(&mut h);
    h.finish()
}

impl LoopGuard {
    pub fn new(cfg: LoopGuardConfig) -> Self {
        let cap = cfg.ping_pong_window.max(2) * 2;
        LoopGuard { cfg, call_counts: HashMap::new(), tool_sequence: VecDeque::with_capacity(cap), per_tool: HashMap::new(), warned: HashSet::new() }
    }

    // may this call go on? (a blocked call is not counted again as a new one)
    pub fn check_call(&mut self, tool: &str, args: &str) -> Verdict {
        if !self.cfg.enabled {
            return Verdict::default();
        }
        let (verdict, key) = self.raw_check(tool, args);
        if verdict.blocked && self.cfg.warn_before_block {
            if self.warned.insert(key) {
                return Verdict { blocked: false, warned: true, reason: verdict.reason };
            }
        }
        verdict
    }

    fn raw_check(&mut self, tool: &str, args: &str) -> (Verdict, String) {
        // 1. identical calls
        let h = hash_call(tool, args);
        let n = self.call_counts.entry(h).or_insert(0);
        *n += 1;
        if *n > self.cfg.max_identical_calls {
            let reason = format!("Identical call to '{}' repeated {} times (max {}).", tool, n, self.cfg.max_identical_calls);
            return (Verdict { blocked: true, warned: false, reason }, format!("identical:{}", h));
        }
        // 2. budget of one tool
        let c = self.per_tool.entry(tool.to_string()).or_insert(0);
        *c += 1;
        if *c > self.cfg.poll_tool_budget {
            let reason = format!("Tool '{}' exceeded its budget ({}).", tool, self.cfg.poll_tool_budget);
            return (Verdict { blocked: true, warned: false, reason }, format!("poll:{}", tool));
        }
        // 3. a repeating pattern
        let cap = self.cfg.ping_pong_window.max(2) * 2;
        if self.tool_sequence.len() == cap {
            self.tool_sequence.pop_front();
        }
        self.tool_sequence.push_back(tool.to_string());
        if self.tool_sequence.len() >= self.cfg.ping_pong_window && self.detect_ping_pong() {
            return (Verdict { blocked: true, warned: false, reason: "Repetitive pattern of calls detected (ping-pong).".to_string() }, "ping_pong".to_string());
        }
        (Verdict::default(), String::new())
    }

    fn detect_ping_pong(&self) -> bool {
        let seq: Vec<&String> = self.tool_sequence.iter().collect();
        let n = seq.len();
        for period in [2usize, 3] {
            if n >= period * 2 {
                let tail = &seq[n - period * 2..];
                if (0..tail.len()).all(|i| tail[i] == tail[i % period]) {
                    return true;
                }
            }
        }
        false
    }

    pub fn reset(&mut self) {
        self.call_counts.clear();
        self.tool_sequence.clear();
        self.per_tool.clear();
        self.warned.clear();
    }
}

#[cfg(test)]
mod tests;
