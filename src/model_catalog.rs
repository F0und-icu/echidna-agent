//! Private, validated catalog data shared by explicit model-selection commands.
//! Wire decoding and authentication remain inside their respective adapters.

use serde::Serialize;

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub(crate) struct CatalogModel {
    pub(crate) slug: String,
    pub(crate) display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) default_reasoning_level: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) supported_reasoning_levels: Vec<CatalogReasoningLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context_window: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_context_window: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) input_modalities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) supports_reasoning_summary_parameter: Option<bool>,
    /// An absent API-key effort field makes no support claim. Codex retains
    /// strict catalog validation, including when its advertised list is empty.
    #[serde(skip)]
    pub(crate) reasoning_levels_advertised: bool,
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub(crate) struct CatalogReasoningLevel {
    pub(crate) effort: String,
    pub(crate) description: String,
}
