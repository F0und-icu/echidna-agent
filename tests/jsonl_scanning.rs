//! Exercise the actual private record scanner without compiling the whole library again.
use echidna::domain;
mod support;

mod project_store {
    use echidna::adapters::project_event_store::ProjectEventStoreError;
    mod jsonl {
        mod scan {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/adapters/project_event_store/jsonl/scan.rs"
            ));
        }

        use super::ProjectEventStoreError;
        use crate::domain::events::{
            EventId, EventSequence, EventUpcasterRegistry, OccurredAt, StreamId,
        };
        use crate::domain::model::ContentItem;
        use crate::domain::session::SessionEvent;
        use std::io::{self, BufRead, BufReader, Cursor, Read, Seek, SeekFrom};
        use std::path::Path;

        fn record(sequence: u64) -> Vec<u8> {
            let event = SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("retained prefix"),
            }
            .into_envelope(
                EventId::new(format!("scan-{sequence}")),
                StreamId::new("scan"),
                EventSequence::new(sequence),
                OccurredAt::new("2026-10-26T00:00:00Z"),
            )
            .unwrap();
            let mut bytes = serde_json::to_vec(&event).unwrap();
            bytes.push(b'\n');
            bytes
        }

        #[test]
        fn tiny_reader_buffers_preserve_exact_envelopes_hash_and_complete_file_behavior() {
            let bytes = [record(1), record(2)].concat();
            let mut reader = BufReader::with_capacity(7, Cursor::new(&bytes));
            let mut validated_hash = blake3::Hasher::new();
            let scan = scan::scan_stream_reader(
                &mut reader,
                Path::new("fixture"),
                &StreamId::new("scan"),
                &EventUpcasterRegistry::current(),
                0,
                None,
                &mut validated_hash,
            )
            .unwrap();
            let scan::StreamScan::Complete {
                events,
                last_sequence,
                validated_bytes,
            } = scan
            else {
                panic!("complete stream was treated as a tail")
            };
            assert_eq!(events.len(), 2);
            assert_eq!(last_sequence.unwrap().get(), 2);
            assert_eq!(validated_bytes, bytes.len());
            assert_eq!(validated_hash.finalize(), blake3::hash(&bytes));
            let temp = crate::support::TemporaryDirectory::new("stream-scanner");
            let path = temp.path().join("events.jsonl");
            std::fs::write(&path, &bytes).unwrap();
            let from_file = scan::scan_stream(
                &path,
                &StreamId::new("scan"),
                &EventUpcasterRegistry::current(),
            )
            .unwrap()
            .into_complete(&path)
            .unwrap();
            assert_eq!(events, from_file);
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }

        #[test]
        fn lexical_tail_middle_corruption_and_unterminated_semantic_failure_remain_distinct() {
            let prefix = record(1);
            let bytes = [prefix.clone(), b"invalid\n".to_vec()].concat();
            let scan = scan::scan_stream_bytes(
                &bytes,
                Path::new("fixture"),
                &StreamId::new("scan"),
                &EventUpcasterRegistry::current(),
            )
            .unwrap();
            let scan::StreamScan::Trailing {
                offset,
                valid_prefix,
                raw_tail,
                last_sequence,
            } = scan
            else {
                panic!("lexical tail must require recovery")
            };
            assert_eq!(offset as usize, prefix.len());
            assert_eq!(valid_prefix, prefix);
            assert_eq!(raw_tail, b"invalid\n");
            assert_eq!(last_sequence.unwrap().get(), 1);
            let middle = [bytes, record(2)].concat();
            assert!(
                matches!(scan::scan_stream_bytes(&middle, Path::new("fixture"), &StreamId::new("scan"), &EventUpcasterRegistry::current()), Err(ProjectEventStoreError::NonTailCorruption { offset, .. }) if offset as usize == prefix.len())
            );
            let mut wrong_sequence = record(3);
            wrong_sequence.pop();
            let semantic = [prefix, wrong_sequence].concat();
            assert!(matches!(
                scan::scan_stream_bytes(
                    &semantic,
                    Path::new("fixture"),
                    &StreamId::new("scan"),
                    &EventUpcasterRegistry::current()
                ),
                Err(ProjectEventStoreError::InvalidEventRecord { .. })
            ));
        }

        struct ChangedDuringRecovery(Cursor<Vec<u8>>);
        impl Read for ChangedDuringRecovery {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.0.read(out)
            }
        }
        impl BufRead for ChangedDuringRecovery {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                self.0.fill_buf()
            }
            fn consume(&mut self, amount: usize) {
                self.0.consume(amount);
            }
        }
        impl Seek for ChangedDuringRecovery {
            fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
                if position == SeekFrom::Start(0) {
                    self.0.get_mut()[0] = b'[';
                }
                self.0.seek(position)
            }
        }
        #[test]
        fn recovery_reread_cannot_replace_the_exact_previously_validated_prefix() {
            let bytes = [record(1), b"invalid\n".to_vec()].concat();
            let mut reader = ChangedDuringRecovery(Cursor::new(bytes));
            assert!(matches!(
                scan::scan_stream_reader(
                    &mut reader,
                    Path::new("fixture"),
                    &StreamId::new("scan"),
                    &EventUpcasterRegistry::current(),
                    0,
                    None,
                    &mut blake3::Hasher::new()
                ),
                Err(ProjectEventStoreError::ValidatedPrefixChanged { .. })
            ));
        }
    }
}
