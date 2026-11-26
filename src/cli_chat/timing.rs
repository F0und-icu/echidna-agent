//! Content-free timing at the existing CLI preparation and request boundaries.

use std::cell::Cell;
use std::time::Instant;

pub(super) struct TurnTiming {
    enabled: bool,
    start: Instant,
    previous: Cell<Instant>,
    first_text: Cell<bool>,
}

impl TurnTiming {
    pub(super) fn new(enabled: bool) -> Self {
        let start = Instant::now();
        Self {
            enabled,
            start,
            previous: Cell::new(start),
            first_text: Cell::new(false),
        }
    }

    pub(super) fn checkpoint(&self, phase: &'static str) {
        if self.enabled {
            let now = Instant::now();
            let previous = self.previous.replace(now);
            let stage_ms = now.duration_since(previous).as_millis();
            let turn_ms = now.duration_since(self.start).as_millis();
            if let Some(stream) = crate::chat_display::JsonLines::current() {
                let _ = stream.send(crate::chat_display::Event::Timing {
                    phase,
                    stage_ms,
                    turn_ms,
                });
                return;
            }
            console_eprintln!("[debug timing] phase={phase} stage_ms={stage_ms} turn_ms={turn_ms}");
        }
    }

    pub(super) fn observe_text(&self, text: &str) {
        if !text.is_empty() && !self.first_text.replace(true) {
            self.checkpoint("first_assistant_text");
        }
    }
}

pub(super) fn task_index_work(enabled: bool, stats: &super::task_index::TaskIndexStats) {
    if enabled && crate::chat_display::JsonLines::current().is_none() {
        console_eprintln!(
            "[debug timing] phase=task_posting_storage posting_file_opens={} posting_file_bytes={} posting_pending_bytes={}",
            stats.posting_file_opens,
            stats.posting_file_bytes,
            stats.posting_pending_bytes
        );
        console_eprintln!(
            "[debug timing] phase=task_index source_rows={} source_prefix_bytes={} source_verification_bytes={} source_index_read_bytes={} source_index_text_write_bytes={} source_index_hash_write_bytes={} source_index_record_write_bytes={} posting_rows_read={} posting_bytes_read={} posting_rows_written={} posting_bytes_written={} cache_rows_written={} cache_bytes_written={} posting_keys={} posting_batch_peak_rows={} posting_batches={} card_rows_read={} card_bytes_read={} task_ref_rows_read={} task_ref_bytes_read={} anchor_rows_read={} anchor_bytes_read={} semantics_rows_read={} semantics_bytes_read={} cards_written={} anchors_written={} navigation_stage_read_bytes={} navigation_stage_write_bytes={} metadata_rebuilt={}",
            stats.source_rows,
            stats.source_prefix_bytes,
            stats.source_verification_bytes,
            stats.source_index_read_bytes,
            stats.source_index_text_write_bytes,
            stats.source_index_hash_write_bytes,
            stats.source_index_record_write_bytes,
            stats.posting_rows_read,
            stats.posting_bytes_read,
            stats.posting_rows_written,
            stats.posting_bytes_written,
            stats.cache_rows_written,
            stats.cache_bytes_written,
            stats.posting_keys,
            stats.posting_batch_peak_rows,
            stats.posting_batches,
            stats.card_rows_read,
            stats.card_bytes_read,
            stats.task_ref_rows_read,
            stats.task_ref_bytes_read,
            stats.anchor_rows_read,
            stats.anchor_bytes_read,
            stats.semantics_rows_read,
            stats.semantics_bytes_read,
            stats.cards_written,
            stats.anchors_written,
            stats.navigation_stage_read_bytes,
            stats.navigation_stage_write_bytes,
            stats.metadata_rebuilt,
        );
    }
}

pub(super) struct ContextReadWork {
    pub(super) scan_bytes: usize,
    pub(super) records: usize,
    pub(super) rank_window_checks: usize,
    pub(super) retrieval_source_window_bytes: usize,
    pub(super) retrieval_index_source_bytes: usize,
    pub(super) retrieval_needle_scan_bytes: usize,
    pub(super) retrieval_candidate_anchors: usize,
    pub(super) retrieval_local_gram_probes: usize,
    pub(super) retrieval_window_gram_probes: usize,
    pub(super) retrieval_rank_window_bytes: usize,
    pub(super) source_verification_bytes: usize,
    pub(super) source_materialization_read_bytes: usize,
    pub(super) source_materialization_write_bytes: usize,
    pub(super) source_index_scan_bytes: usize,
    pub(super) source_index_text_write_bytes: usize,
    pub(super) source_index_hash_write_bytes: usize,
    pub(super) source_index_record_write_bytes: usize,
    pub(super) auxiliary_preparation_memory_read_bytes: usize,
    pub(super) auxiliary_preparation_write_bytes: usize,
    pub(super) indexed_events: usize,
    pub(super) indexed_bytes: usize,
    pub(super) initial_indexed_events: usize,
    pub(super) initial_indexed_bytes: usize,
}

pub(super) fn context_read_work(enabled: bool, work: &ContextReadWork) {
    if enabled && crate::chat_display::JsonLines::current().is_none() {
        console_eprintln!(
            "[debug timing] phase=context_read scan_bytes={} records={} rank_window_checks={} retrieval_source_window_bytes={} retrieval_index_source_bytes={} retrieval_needle_scan_bytes={} retrieval_candidate_anchors={} retrieval_local_gram_probes={} retrieval_window_gram_probes={} retrieval_rank_window_bytes={} source_verification_bytes={} source_materialization_read_bytes={} source_materialization_write_bytes={} source_index_scan_bytes={} source_index_text_write_bytes={} source_index_hash_write_bytes={} source_index_record_write_bytes={} auxiliary_preparation_memory_read_bytes={} auxiliary_preparation_write_bytes={} indexed_events={} indexed_bytes={} initial_indexed_events={} initial_indexed_bytes={}",
            work.scan_bytes,
            work.records,
            work.rank_window_checks,
            work.retrieval_source_window_bytes,
            work.retrieval_index_source_bytes,
            work.retrieval_needle_scan_bytes,
            work.retrieval_candidate_anchors,
            work.retrieval_local_gram_probes,
            work.retrieval_window_gram_probes,
            work.retrieval_rank_window_bytes,
            work.source_verification_bytes,
            work.source_materialization_read_bytes,
            work.source_materialization_write_bytes,
            work.source_index_scan_bytes,
            work.source_index_text_write_bytes,
            work.source_index_hash_write_bytes,
            work.source_index_record_write_bytes,
            work.auxiliary_preparation_memory_read_bytes,
            work.auxiliary_preparation_write_bytes,
            work.indexed_events,
            work.indexed_bytes,
            work.initial_indexed_events,
            work.initial_indexed_bytes
        );
    }
}
