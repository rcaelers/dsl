//! Explicit compatibility validation against an upstream Sigrok SPI decoder.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver as ChannelReceiver, bounded};

use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokChannel, SigrokDecoder, SigrokDecoderConfig, SigrokInitialPin, SigrokOptionValue,
};
use signal_capture::SampleBlock;
use signal_derived::{ProtocolPacket, ProtocolValue, Word, WordPayload};
use signal_runtime::{
    ChannelMessage, InputPort, OutputPort, ProcessNode, Sender, Watchdog, WorkError, WorkExecutor,
    WorkExecutorTask, WorkTask,
};

use super::execution::PythonSigrokExecutionFactory;

#[derive(Debug, PartialEq)]
struct SpiResult {
    annotations: Vec<(u64, u64, u64, String)>,
    binary: Vec<(u64, Vec<u8>)>,
    metadata: Vec<(u64, String)>,
    packets: Vec<(String, ProtocolValue)>,
}

struct ValidationWorkExecutor;

impl WorkExecutor for ValidationWorkExecutor {
    fn available_parallelism(&self) -> usize {
        1
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        self.submit_long_running(task)
    }

    fn submit_long_running(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        Ok(Box::new(ValidationWorkTask {
            handle: Some(std::thread::spawn(task)),
        }))
    }
}

struct ValidationWorkTask {
    handle: Option<JoinHandle<()>>,
}

impl WorkTask for ValidationWorkTask {
    fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn wait(mut self: Box<Self>) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Verifies that the upstream SPI decoder is invariant across every input chunk boundary.
///
/// # Parameters
/// - `decoder_root`: Input consumed by this operation.
pub fn validate_spi_chunk_boundaries(decoder_root: &Path) -> Result<(), String> {
    validate_decoder_root(decoder_root)?;
    let signals = spi_signals(0xa5);
    let reference = run_spi(decoder_root, &signals, &[signals[0].len()])?;
    if !reference
        .annotations
        .iter()
        .any(|(_, _, class, text)| *class == 1 && text == "A5")
    {
        return Err("upstream SPI decoder produced no expected A5 annotation".into());
    }
    if !reference
        .binary
        .iter()
        .any(|(class, bytes)| *class == 1 && bytes == &[0xa5])
    {
        return Err("upstream SPI decoder produced no expected A5 binary output".into());
    }
    if !reference
        .metadata
        .iter()
        .any(|(_, label)| label.starts_with("Bitrate:"))
    {
        return Err("upstream SPI decoder produced no bitrate metadata".into());
    }
    if !reference.packets.iter().any(|(protocol, value)| {
        protocol == "spi"
            && matches!(value, ProtocolValue::List(items) if matches!(items.first(), Some(ProtocolValue::String(kind)) if kind == "DATA"))
    }) {
        return Err("upstream SPI decoder produced no expected protocol packet".into());
    }

    for boundary in 1..signals[0].len() {
        let chunked = run_spi(
            decoder_root,
            &signals,
            &[boundary, signals[0].len() - boundary],
        )?;
        if chunked != reference {
            return Err(format!("SPI output changed at chunk boundary {boundary}"));
        }
    }
    println!(
        "Sigrok SPI chunk-boundary validation passed across {} boundaries",
        signals[0].len() - 1
    );
    Ok(())
}

/// Compares the hosted SPI decoder output with an installed libsigrokdecode oracle.
pub fn validate_spi_oracle(
    decoder_root: &Path,
    pkg_config_name: &str,
    c_compiler: &str,
) -> Result<(), String> {
    validate_decoder_root(decoder_root)?;
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", pkg_config_name])
        .output()
        .map_err(|error| format!("could not run pkg-config: {error}"))?;
    if !flags.status.success() {
        return Err(format!(
            "pkg-config could not resolve {pkg_config_name}: {}",
            String::from_utf8_lossy(&flags.stderr)
        ));
    }
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let source = directory.path().join("oracle.c");
    let executable = directory.path().join("sigrok-oracle");
    std::fs::write(&source, include_str!("oracle.c")).map_err(|error| error.to_string())?;
    let mut compiler = Command::new(c_compiler);
    compiler.arg(&source).arg("-o").arg(&executable);
    compiler.args(
        String::from_utf8(flags.stdout)
            .map_err(|error| error.to_string())?
            .split_whitespace(),
    );
    let compiled = compiler
        .output()
        .map_err(|error| format!("could not run C compiler '{c_compiler}': {error}"))?;
    if !compiled.status.success() {
        return Err(format!(
            "could not build libsigrokdecode oracle:\n{}",
            String::from_utf8_lossy(&compiled.stderr)
        ));
    }
    let oracle = Command::new(executable)
        .arg(decoder_root)
        .output()
        .map_err(|error| format!("could not run libsigrokdecode oracle: {error}"))?;
    if !oracle.status.success() {
        return Err(format!(
            "libsigrokdecode oracle failed:\n{}",
            String::from_utf8_lossy(&oracle.stderr)
        ));
    }

    let host = run_spi(decoder_root, &spi_signals(0xa5), &[19])?;
    let host_annotations = host
        .annotations
        .iter()
        .map(|(start, end, class, text)| format!("A {start} {end} {class} {text}"))
        .chain(host.binary.iter().map(|(class, bytes)| {
            let value = bytes
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>();
            let annotation = host
                .annotations
                .iter()
                .find(|(_, _, annotation_class, _)| {
                    (*class == 0 && *annotation_class == 0)
                        || (*class == 1 && *annotation_class == 1)
                })
                .expect("binary output has no matching annotation span");
            format!("B {} {} {class} {value}", annotation.0, annotation.1)
        }))
        .collect::<HashSet<_>>();
    let oracle = String::from_utf8(oracle.stdout)
        .map_err(|error| error.to_string())?
        .lines()
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    if host_annotations != oracle {
        return Err(format!(
            "hosted decoder output differs from libsigrokdecode:\nhost={host_annotations:#?}\noracle={oracle:#?}"
        ));
    }
    println!("Sigrok SPI output matches the installed libsigrokdecode oracle");
    Ok(())
}

fn validate_decoder_root(decoder_root: &Path) -> Result<(), String> {
    if decoder_root.join("spi/pd.py").is_file() {
        Ok(())
    } else {
        Err(format!(
            "decoder directory '{}' does not contain spi/pd.py",
            decoder_root.display()
        ))
    }
}

fn run_spi(
    decoder_root: &Path,
    signals: &[Vec<bool>; 3],
    chunks: &[usize],
) -> Result<SpiResult, String> {
    let watchdog = Watchdog::new();
    let inputs = signals
        .iter()
        .enumerate()
        .map(|(channel, samples)| block_input(&watchdog, samples, chunks, channel))
        .collect::<Result<Vec<_>, _>>()?;
    let (annotation_output, annotation_receiver) = output::<Word>(&watchdog, 0);
    let (binary_output, binary_receiver) = output::<Word>(&watchdog, 1);
    let (logic_output, _logic_receiver) = output::<SampleBlock>(&watchdog, 2);
    let (metadata_output, metadata_receiver) = output::<Word>(&watchdog, 3);
    let (packet_output, packet_receiver) = output::<ProtocolPacket>(&watchdog, 4);
    let outputs = vec![
        annotation_output,
        binary_output,
        logic_output,
        metadata_output,
        packet_output,
    ];
    let executor: Arc<dyn WorkExecutor> = Arc::new(ValidationWorkExecutor);
    let mut decoder = SigrokDecoder::with_execution_factory(
        spi_config(decoder_root),
        &PythonSigrokExecutionFactory::new(executor),
    )?;
    loop {
        match decoder.work(&inputs, &outputs) {
            Ok(_) if decoder.should_stop() => break,
            Ok(_) => {}
            Err(WorkError::Shutdown) => break,
            Err(error) => return Err(format!("unexpected Sigrok node error: {error}")),
        }
    }
    Ok(SpiResult {
        annotations: collect(annotation_receiver)
            .into_iter()
            .map(|value| {
                (
                    value.timestamp_ns,
                    value.end_ns(),
                    value.value,
                    word_text(&value),
                )
            })
            .collect(),
        binary: collect(binary_receiver)
            .into_iter()
            .map(|value| (value.value, word_bytes(&value)))
            .collect(),
        metadata: collect(metadata_receiver)
            .into_iter()
            .map(|value| (value.value, word_text(&value)))
            .collect(),
        packets: collect(packet_receiver)
            .into_iter()
            .map(|value| (value.protocol_id, value.value))
            .collect(),
    })
}

fn spi_config(decoder_root: &Path) -> SigrokDecoderConfig {
    SigrokDecoderConfig {
        decoder_root: decoder_root.to_owned(),
        decoder_id: "spi".into(),
        sample_rate: 1_000_000_000,
        channels: vec![
            SigrokChannel {
                name: "clk".into(),
                connected: true,
                initial_pin: SigrokInitialPin::SameAsFirstSample,
            },
            SigrokChannel {
                name: "miso".into(),
                connected: false,
                initial_pin: SigrokInitialPin::SameAsFirstSample,
            },
            SigrokChannel {
                name: "mosi".into(),
                connected: true,
                initial_pin: SigrokInitialPin::SameAsFirstSample,
            },
            SigrokChannel {
                name: "cs".into(),
                connected: true,
                initial_pin: SigrokInitialPin::SameAsFirstSample,
            },
        ],
        protocol_inputs: Vec::new(),
        options: BTreeMap::from([
            (
                "bitorder".into(),
                SigrokOptionValue::String("msb-first".into()),
            ),
            (
                "cs_polarity".into(),
                SigrokOptionValue::String("active-low".into()),
            ),
            ("cpol".into(), SigrokOptionValue::Integer(0)),
            ("cpha".into(), SigrokOptionValue::Integer(0)),
            ("wordsize".into(), SigrokOptionValue::Integer(8)),
        ]),
        annotation_rows_by_class: vec![
            Arc::from([1]),
            Arc::from([4]),
            Arc::from([0]),
            Arc::from([3]),
            Arc::from([6]),
            Arc::from([2]),
            Arc::from([5]),
        ],
        binary_class_count: 2,
        logic_groups: Vec::new(),
    }
}

fn spi_signals(word: u8) -> [Vec<bool>; 3] {
    let mut clock = vec![false, false];
    let mut mosi = vec![word & 0x80 != 0; 2];
    let mut chip_select = vec![true, false];
    for bit in (0..8).rev() {
        let value = word & (1 << bit) != 0;
        clock.extend([true, false]);
        mosi.extend([value, value]);
        chip_select.extend([false, false]);
    }
    clock.push(false);
    mosi.push(word & 1 != 0);
    chip_select.push(true);
    [clock, mosi, chip_select]
}

fn block_input(
    watchdog: &Watchdog,
    samples: &[bool],
    chunks: &[usize],
    channel: usize,
) -> Result<InputPort, String> {
    let (sender, receiver) = bounded(8);
    let mut start = 0;
    for &count in chunks {
        let end = start + count;
        let selected = samples
            .get(start..end)
            .ok_or_else(|| format!("chunk layout exceeds channel {channel} input"))?;
        sender
            .send(ChannelMessage::Sample(SampleBlock::new(
                pack(selected),
                start as u64,
                count,
                1,
            )))
            .map_err(|error| error.to_string())?;
        start = end;
    }
    if start != samples.len() {
        return Err(format!(
            "chunk layout covers {start} of {} channel {channel} samples",
            samples.len()
        ));
    }
    drop(sender);
    Ok(InputPort::new_with_watchdog(
        receiver,
        watchdog,
        "sigrok-validation",
        &format!("in{channel}"),
    ))
}

fn output<T: Clone + Send + 'static>(
    watchdog: &Watchdog,
    index: usize,
) -> (OutputPort, ChannelReceiver<ChannelMessage<T>>) {
    let (sender, receiver) = bounded(1_024);
    (
        OutputPort::new_with_watchdog(
            Sender::new(vec![sender]),
            watchdog,
            "sigrok-validation",
            &format!("out{index}"),
        ),
        receiver,
    )
}

fn collect<T>(receiver: ChannelReceiver<ChannelMessage<T>>) -> Vec<T> {
    receiver
        .try_iter()
        .flat_map(|message| match message {
            ChannelMessage::Sample(value) => vec![value],
            ChannelMessage::Batch(values) => values,
            ChannelMessage::EndOfStream => Vec::new(),
        })
        .collect()
}

fn pack(samples: &[bool]) -> Arc<[u8]> {
    let mut packed = vec![0_u8; samples.len().div_ceil(8)];
    for (sample, high) in samples.iter().copied().enumerate() {
        if high {
            packed[sample / 8] |= 1 << (sample % 8);
        }
    }
    packed.into()
}

fn word_text(word: &Word) -> String {
    match word.payload.as_ref() {
        Some(WordPayload::Text(text)) => text.to_string(),
        Some(WordPayload::Bytes(bytes)) => bytes
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" "),
        None => word.value.to_string(),
    }
}

fn word_bytes(word: &Word) -> Vec<u8> {
    match word.payload.as_ref() {
        Some(WordPayload::Bytes(bytes)) => bytes.to_vec(),
        _ => word.value.to_be_bytes().to_vec(),
    }
}
