// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
// Its own test target with one test: the failpoint state it arms is process-wide, and no other
// test shares this process.

//! A journal append that fails at each of its durability points: the rejected operations leave
//! no episode and consume no episode uid, in memory or after a reopen, and the occurrence
//! references acknowledged before the failure keep addressing the same nodes.
#![cfg(feature = "failpoints")]

use std::path::Path;

use serde_json::json;
use writerslogic_warm::core::experience::{OccurrenceNode, OccurrenceRef, OwnedObservation};
use writerslogic_warm::core::failpoint::{arm, disarm, hits, Fault};
use writerslogic_warm::WarmCore;

const DIM: u32 = 4096;

fn obs(ep: &str, n: u32) -> OwnedObservation {
    let row = |who: &str, h: &str| {
        json!({"role": "row", "name": who,
               "children": [{"role": "button", "name": "Delete", "handle": h}]})
    };
    let mut o = json!({
        "episode": ep, "step": n, "url": "http://h/t",
        "tree": {"role": "RootWebArea", "name": format!("Tickets {ep}"),
                 "children": [row("INC1", "7"), row("INC2", "9")]},
    });
    if n > 0 {
        o["action"] = json!({"call": "click('9')"});
    }
    serde_json::from_value(o).unwrap()
}

fn core(dir: &Path) -> WarmCore {
    WarmCore::new(DIM, Some(dir.display().to_string()), None).unwrap()
}

type Held = (u64, OccurrenceRef, OccurrenceNode);

fn held(c: &WarmCore) -> Held {
    c.with_experience(|s| {
        let r = s.action_targets("a", 1).unwrap().targets[0]
            .1
            .clone()
            .unwrap();
        (
            s.episode("a").unwrap().uid,
            r.clone(),
            s.occurrence(&r).unwrap(),
        )
    })
    .unwrap()
}

fn check(c: &WarmCore, want: &Held, context: &str) {
    c.with_experience(|s| {
        assert!(
            !s.has_episode("b"),
            "{context}: the rejected episode is stored"
        );
        assert!(
            !s.has_step("a", 2),
            "{context}: the rejected step is stored"
        );
        assert_eq!(
            s.episode("a").unwrap().uid,
            want.0,
            "{context}: uid changed"
        );
        assert_eq!(
            s.occurrence(&want.1).as_ref(),
            Some(&want.2),
            "{context}: the reference moved"
        );
    })
    .unwrap();
}

#[test]
fn a_failed_journal_append_changes_no_identity_and_keeps_acknowledged_references() {
    let dir = tempfile::tempdir().unwrap();
    let c = core(dir.path());
    disarm();
    c.with_experience_mut(|s| s.ingest_many(vec![obs("a", 0), obs("a", 1)]).unwrap())
        .unwrap();
    let want = held(&c);
    assert_eq!(want.1.path, vec![1, 0]);

    // The durability points one uncut batch passes, counted on a copy of the workload.
    let probe = tempfile::tempdir().unwrap();
    let p = core(probe.path());
    p.with_experience_mut(|s| s.ingest_many(vec![obs("a", 0), obs("a", 1)]).unwrap())
        .unwrap();
    disarm();
    p.with_experience_mut(|s| s.ingest_many(vec![obs("b", 0), obs("a", 2)]).unwrap())
        .unwrap();
    let points = hits();
    drop(p);
    assert!(points >= 3, "append, write and sync: {points}");

    let mut c = c;
    for at in 1..=points {
        for fault in [Fault::Io, Fault::NoSpace] {
            let context = format!("point {at} {fault:?}");
            arm(fault, at, false);
            let result = c
                .with_experience_mut(|s| s.ingest_many(vec![obs("b", 0), obs("a", 2)]))
                .unwrap();
            disarm();
            assert!(result.is_err(), "{context}: the batch was acknowledged");
            check(&c, &want, &context);
            drop(c);
            c = core(dir.path());
            check(&c, &want, &format!("{context}, reopened"));
        }
    }

    // The failures consumed no uid: the next new episode takes the one after `a`.
    c.with_experience_mut(|s| s.ingest_many(vec![obs("b", 0), obs("a", 2)]).unwrap())
        .unwrap();
    let b = c.with_experience(|s| s.episode("b").unwrap().uid).unwrap();
    assert_eq!(b, want.0 + 1);
    drop(c);
    let c = core(dir.path());
    let (b_again, a) = c
        .with_experience(|s| (s.episode("b").unwrap().uid, s.has_step("a", 2)))
        .unwrap();
    assert_eq!((b_again, a), (b, true));
    let again = held(&c);
    assert_eq!(again, want);
}
