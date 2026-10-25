//! Exercise the private sample owner once, without widening its production API.
use echidna::domain;
#[path = "../src/app/model_sample.rs"]
mod model_sample;

use domain::model::{ContentItem, ModelEvent, ModelUsage};

#[test]
fn model_sample_retains_final_usage_separately_from_history_and_assistant_text() {
    let usage = ModelUsage {
        input_tokens: 1_000,
        output_tokens: 200,
        cached_input_tokens: Some(750),
        reasoning_output_tokens: Some(150),
    };
    let mut first = usage;
    first.input_tokens = 2_000;
    let sample = model_sample::ModelSample::from_events(vec![
        ModelEvent::Usage { usage: first },
        ModelEvent::TextDelta {
            text: "Answer.".to_owned(),
        },
        ModelEvent::Usage { usage },
    ]);
    assert_eq!(sample.usage(), Some(usage));
    let mut history = domain::history::ConversationHistory::default();
    sample.commit(&mut history).unwrap();
    assert_eq!(history.items(), &[ContentItem::assistant_text("Answer.")]);
    assert_eq!(sample.assistant_text(), "Answer.");
    assert!(sample.tool_calls().is_empty());
    assert_eq!(sample.content_items(), history.items());
    assert_eq!(
        model_sample::ModelSample::from_events(Vec::new()).usage(),
        None
    );
}
