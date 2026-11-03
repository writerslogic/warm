#[test]
fn hashed_owner_limit_rejects_before_consuming_and_preserves_membership() {
    for count in [DENSE_SLOT_LIMIT - 1, DENSE_SLOT_LIMIT, DENSE_SLOT_LIMIT + 1] {
        let consumed = std::cell::Cell::new(0);
        let owners =
            EligibleOwners::hashed((0..count).inspect(|_| consumed.set(consumed.get() + 1)));
        if count > DENSE_SLOT_LIMIT {
            assert!(owners.is_none());
            assert_eq!(consumed.get(), 0);
        } else {
            let owners = owners.expect("bounded owner set rejected");
            assert_eq!(consumed.get(), count);
            assert!(owners.contains(&0));
            assert!(owners.contains(&(count - 1)));
            assert!(!owners.contains(&count));
            assert!(!owners.contains(&usize::MAX));
        }
    }
}
