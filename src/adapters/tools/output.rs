//! One bounded text-output policy for built-in Tool adapters.

pub(super) struct BoundedText {
    pub text: String,
    pub original_bytes: usize,
    pub truncated: bool,
}

pub(super) fn bounded_text(bytes: &[u8], max_bytes: usize) -> BoundedText {
    let original_bytes = bytes.len();
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    let truncated = text.len() > max_bytes;
    if truncated {
        let mut end = max_bytes.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    BoundedText {
        text,
        original_bytes,
        truncated,
    }
}
