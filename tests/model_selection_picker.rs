//! No-network regression coverage for the private auth selection picker.
//!
//! Compile the library as this test crate so it can exercise the crate-private
//! picker helpers without widening the product API.

include!("../src/lib.rs");

mod picker_regression {
    use std::io::Cursor;

    use crate::adapters::subscription_auth::{SubscriptionModel, SubscriptionReasoningLevel};
    use crate::cli_auth::{read_selection_choice, select_effort_choice, select_model_choice};
    use crate::domain::model::ReasoningEffort;

    fn catalog() -> Vec<SubscriptionModel> {
        vec![
            SubscriptionModel {
                slug: "first".to_owned(),
                display_name: "First".to_owned(),
                default_reasoning_level: None,
                supported_reasoning_levels: vec![
                    SubscriptionReasoningLevel {
                        effort: "medium".to_owned(),
                        description: "Balanced".to_owned(),
                    },
                    SubscriptionReasoningLevel {
                        effort: "high".to_owned(),
                        description: "Thorough".to_owned(),
                    },
                    SubscriptionReasoningLevel {
                        effort: "future".to_owned(),
                        description: "Server-defined".to_owned(),
                    },
                ],
            },
            SubscriptionModel {
                slug: "second".to_owned(),
                display_name: "Second".to_owned(),
                default_reasoning_level: None,
                supported_reasoning_levels: Vec::new(),
            },
        ]
    }

    #[test]
    fn numbered_model_and_effort_choices_select_only_catalog_values() {
        let catalog = catalog();
        assert_eq!(select_model_choice(&catalog, "2").unwrap(), "second");
        assert_eq!(select_model_choice(&catalog, "first").unwrap(), "first");
        assert_eq!(
            select_effort_choice(&catalog[0], Some("2")).unwrap(),
            Some(ReasoningEffort::High)
        );
        assert_eq!(
            select_effort_choice(&catalog[0], Some("medium")).unwrap(),
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(select_effort_choice(&catalog[0], Some("0")).unwrap(), None);
    }

    #[test]
    fn picker_rejects_invalid_or_advertised_but_unknown_choices() {
        let catalog = catalog();
        let model_error = select_model_choice(&catalog, "3").unwrap_err();
        assert!(
            model_error
                .to_string()
                .contains("outside the current catalog")
        );
        let effort_error = select_effort_choice(&catalog[0], Some("4")).unwrap_err();
        assert!(
            effort_error
                .to_string()
                .contains("outside the current catalog")
        );
        let unknown_error = select_effort_choice(&catalog[0], Some("future")).unwrap_err();
        assert!(
            unknown_error
                .to_string()
                .contains("advertised but not executable")
        );
    }

    #[test]
    fn eof_cancels_selection_and_empty_effort_keeps_service_default() {
        let error = read_selection_choice(&mut Cursor::new(Vec::<u8>::new())).unwrap_err();
        assert!(error.to_string().contains("selection was cancelled"));
        let catalog = catalog();
        let blank = read_selection_choice(&mut Cursor::new(b"\n".to_vec())).unwrap();
        assert_eq!(blank, None);
        assert_eq!(
            select_effort_choice(&catalog[0], blank.as_deref()).unwrap(),
            None
        );
    }
}
