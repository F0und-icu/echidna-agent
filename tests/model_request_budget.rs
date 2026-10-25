//! Admission counts and optional capacity through the existing private owner.
include!("../src/lib.rs");

use crate::app::model_request_budget::{DEFAULT_MODEL_REQUEST_LIMIT, ModelRequestBudget};
use crate::domain::context::ProviderSessionRole;

#[test]
fn unlimited_budget_counts_primary_and_vision_without_a_finite_replacement_cap() {
    let budget = ModelRequestBudget::new(0);
    assert_eq!(budget.limit(), None);
    for index in 0..10_005 {
        let role = if index % 2 == 0 {
            ProviderSessionRole::Primary
        } else {
            ProviderSessionRole::Vision
        };
        assert_eq!(budget.admit(role), Ok(()));
    }
    assert_eq!(budget.remaining(), None);
    assert_eq!(budget.counts().primary, 5_003);
    assert_eq!(budget.counts().vision, 5_002);
    assert_eq!(budget.counts().total(), 10_005);
}

#[test]
fn default_and_positive_budgets_keep_exact_shared_exhaustion_and_counts() {
    assert_eq!(DEFAULT_MODEL_REQUEST_LIMIT, 32);
    for limit in [1, DEFAULT_MODEL_REQUEST_LIMIT, 10_000] {
        let budget = ModelRequestBudget::new(limit);
        assert_eq!(budget.limit(), Some(limit));
        assert_eq!(budget.remaining(), Some(limit));
        for index in 0..limit {
            let role = if index % 2 == 0 {
                ProviderSessionRole::Primary
            } else {
                ProviderSessionRole::Vision
            };
            assert_eq!(budget.admit(role), Ok(()));
            assert_eq!(budget.remaining(), Some(limit - index - 1));
        }
        for role in [ProviderSessionRole::Primary, ProviderSessionRole::Vision] {
            assert_eq!(budget.admit(role), Err(limit));
        }
        assert_eq!(budget.counts().total(), limit);
        assert_eq!(budget.remaining(), Some(0));
    }
}
