//! Generated-workload benchmark for the indexed derived-word store.

std::cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        fn main() -> Result<(), Box<dyn std::error::Error>> {
            native::main()
        }

        mod native {
            use std::time::Instant;

            use signal_processing::{IndexedAnnotationWriter, LiveStoreConfig, Word};

            const BATCH_WORDS: usize = 32_768;
            const TOTAL_WORDS: usize = BATCH_WORDS * 64;
            const MINIMUM_WORDS_PER_SECOND: f64 = 20_000_000.0;

            pub(crate) fn main() -> Result<(), Box<dyn std::error::Error>> {
                let directory = tempfile::tempdir()?;
                let config = LiveStoreConfig {
                    directory: directory.path().to_owned(),
                    ..LiveStoreConfig::default()
                };
                let words = (0..TOTAL_WORDS)
                    .map(|index| Word::new((index & 0xff) as u64, index as u64 * 80))
                    .collect::<Vec<_>>();
                let (mut writer, store) = IndexedAnnotationWriter::create(config)?;

                let started = Instant::now();
                for batch in words.chunks(BATCH_WORDS) {
                    writer.append_batch(batch)?;
                }
                writer.finish()?;
                let elapsed = started.elapsed();
                let throughput = TOTAL_WORDS as f64 / elapsed.as_secs_f64();
                let metadata = store.snapshot().metadata;

                println!(
                    "derived-word-store words={TOTAL_WORDS} blocks={} bytes={} elapsed_s={:.3} words_s={throughput:.1}",
                    metadata.committed_block_count,
                    metadata.committed_data_len,
                    elapsed.as_secs_f64(),
                );
                assert_eq!(metadata.committed_word_count, TOTAL_WORDS as u64);
                assert!(
                    throughput >= MINIMUM_WORDS_PER_SECOND,
                    "derived-word store encoded {throughput:.1} words/s; expected at least \
                     {MINIMUM_WORDS_PER_SECOND:.1}"
                );
                Ok(())
            }
        }
    }
}
