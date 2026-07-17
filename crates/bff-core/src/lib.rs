//! bff-core: domain-agnostic aggregation/composition logic and this repo's own
//! aggregates and DTOs (see ADR-004, ../ddd/consultant-experience-context.md).
//! Depends only on nexus-client's and persistence's trait interfaces, never their
//! concrete implementations.

#[cfg(test)]
mod tests {
    // PROMPT-05 placeholder: bff-core has no aggregate logic yet (that lands in
    // U20/U21/U22), so this is a harness smoke test, not a real behavioral test —
    // it exists only to prove `cargo test --workspace` exercises this crate.
    #[test]
    fn it_compiles_and_runs() {
        assert_eq!(1 + 1, 2);
    }
}
