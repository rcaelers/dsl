//! Word matcher emitting a trigger whenever a decoded word matches a pattern.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tracing::debug;

use signal_processing::{Sample, Trigger, Word};
use signal_runtime::{
    ConfigOutcome, ConfigValue, ConfigurationBoundary, ConfigurationScheduler, InputPort,
    NodeConfig, OutputPort, PortDirection, PortSchema, ProcessNode, WorkError, WorkOutcome,
    WorkResult,
};

/// Comparison applied between the masked word and the masked pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchOp {
    #[default]
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl MatchOp {
    fn matches(&self, word: u64, pattern: u64) -> bool {
        match self {
            MatchOp::Eq => word == pattern,
            MatchOp::Ne => word != pattern,
            MatchOp::Lt => word < pattern,
            MatchOp::Le => word <= pattern,
            MatchOp::Gt => word > pattern,
            MatchOp::Ge => word >= pattern,
        }
    }

    /// Parse from the wire names used by node configs ("eq", "ne", …).
    ///
    /// # Parameters
    /// - `name`: Input consumed by this operation.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "eq" => MatchOp::Eq,
            "ne" => MatchOp::Ne,
            "lt" => MatchOp::Lt,
            "le" => MatchOp::Le,
            "gt" => MatchOp::Gt,
            "ge" => MatchOp::Ge,
            _ => return None,
        })
    }
}

/// Where in the matched word the emitted [`Trigger`] lands. A command
/// logically takes effect once it has fully arrived, so `End` (the word's
/// last sampling edge, `Word::end_ns`) is the default; for instantaneous
/// words (`duration_ns == 0`) the two coincide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerAt {
    Start,
    #[default]
    End,
}

impl TriggerAt {
    /// Parse from the wire names used by node configs.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "start" => TriggerAt::Start,
            "end" => TriggerAt::End,
            _ => return None,
        })
    }
}

/// Predicate family applied to the masked numeric word value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PredicateMode {
    #[default]
    Compare,
    InclusiveRange,
    Set,
}

impl PredicateMode {
    /// Parses a configured word literal into the matcher representation.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "compare" => Self::Compare,
            "range" => Self::InclusiveRange,
            "set" => Self::Set,
            _ => return None,
        })
    }
}

/// Emits a [`Trigger`] for every word where `(word & mask) OP (pattern &
/// mask)` holds (`OP` = [`MatchOp`], `==` by default). The trigger lands at
/// the word's end by default ([`TriggerAt`]).
///
/// Inputs: `words` — a [`Word`] stream (from any decoder — SPI, parallel
/// bus, UART, … — the matcher has no notion of which one)
/// Outputs: `trigger` — `Trigger` per match;
///          `matched` — optional `Sample` pulse lane for visualization
pub struct WordMatcher {
    name: String,
    pattern: u64,
    mask: u64,
    op: MatchOp,
    trigger_at: TriggerAt,
    predicate_mode: PredicateMode,
    range_min: u64,
    range_max: u64,
    set_values: Vec<u64>,
    match_count: u64,
    holdoff_ns: u64,
    manual_rearm: bool,
    armed: bool,
    next_allowed_ns: u64,
    eligible_matches: u64,
    /// Width of the visualization pulse on the `matched` output.
    pulse_ns: u64,
    input_buffer: VecDeque<Word>,
    rearm_buffer: VecDeque<Trigger>,
    word_head: Option<Word>,
    rearm_head: Option<Trigger>,
    word_eos: bool,
    rearm_eos: bool,
    matches: u64,
    /// End of the previously emitted pulse (monotonicity guard).
    last_pulse_end: u64,
    started: bool,
    scheduled_settings: Arc<Mutex<VecDeque<(ConfigurationBoundary, NodeConfig)>>>,
}

#[derive(Clone, Default)]
struct MatcherSettings {
    pattern: u64,
    mask: u64,
    op: MatchOp,
    trigger_at: TriggerAt,
    predicate_mode: PredicateMode,
    range_min: u64,
    range_max: u64,
    set_values: Vec<u64>,
    match_count: u64,
    holdoff_ns: u64,
}

struct WordMatcherConfigurationScheduler {
    scheduled: Arc<Mutex<VecDeque<(ConfigurationBoundary, NodeConfig)>>>,
}

impl ConfigurationScheduler for WordMatcherConfigurationScheduler {
    fn schedule_config(
        &self,
        config: &NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> ConfigOutcome {
        if WordMatcher::configured_settings(MatcherSettings::default(), config).is_err() {
            return ConfigOutcome::NeedsRestart;
        }
        let mut scheduled = self
            .scheduled
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if scheduled
            .back()
            .is_some_and(|(previous, _)| previous.timestamp_ns > boundary.timestamp_ns)
        {
            return ConfigOutcome::NeedsRestart;
        }
        scheduled.push_back((boundary, config.clone()));
        ConfigOutcome::Applied
    }
}

impl WordMatcher {
    /// Creates a word matcher with the supplied comparison configuration.
    ///
    /// # Parameters
    /// - `pattern`: Input consumed by this operation.
    /// - `mask`: Input consumed by this operation.
    pub fn new(pattern: u64, mask: u64) -> Self {
        Self {
            name: "word_matcher".to_string(),
            pattern,
            mask,
            op: MatchOp::default(),
            trigger_at: TriggerAt::default(),
            predicate_mode: PredicateMode::default(),
            range_min: 0,
            range_max: u64::MAX,
            set_values: Vec::new(),
            match_count: 1,
            holdoff_ns: 0,
            manual_rearm: false,
            armed: true,
            next_allowed_ns: 0,
            eligible_matches: 0,
            pulse_ns: 1_000,
            input_buffer: VecDeque::new(),
            rearm_buffer: VecDeque::new(),
            word_head: None,
            rearm_head: None,
            word_eos: false,
            rearm_eos: false,
            matches: 0,
            last_pulse_end: 0,
            started: false,
            scheduled_settings: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Returns this value configured with name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Returns this value configured with op.
    pub fn with_op(mut self, op: MatchOp) -> Self {
        self.op = op;
        self
    }

    /// Returns this value configured with trigger at.
    pub fn with_trigger_at(mut self, trigger_at: TriggerAt) -> Self {
        self.trigger_at = trigger_at;
        self
    }

    /// Returns this value configured with inclusive range.
    pub fn with_inclusive_range(mut self, minimum: u64, maximum: u64) -> Self {
        self.predicate_mode = PredicateMode::InclusiveRange;
        self.range_min = minimum;
        self.range_max = maximum;
        self
    }

    /// Returns this value configured with set.
    pub fn with_set(mut self, values: Vec<u64>) -> Self {
        self.predicate_mode = PredicateMode::Set;
        self.set_values = values;
        self
    }

    /// Returns this value configured with match count.
    pub fn with_match_count(mut self, match_count: u64) -> Self {
        self.match_count = match_count.max(1);
        self
    }

    /// Returns this value configured with holdoff ns.
    pub fn with_holdoff_ns(mut self, holdoff_ns: u64) -> Self {
        self.holdoff_ns = holdoff_ns;
        self
    }

    /// Returns this value configured with manual rearm.
    pub fn with_manual_rearm(mut self, manual_rearm: bool) -> Self {
        self.manual_rearm = manual_rearm;
        self
    }

    /// Returns this value configured with pulse ns.
    ///
    /// # Parameters
    /// - `pulse_ns`: Input consumed by this operation.
    pub fn with_pulse_ns(mut self, pulse_ns: u64) -> Self {
        self.pulse_ns = pulse_ns.max(1);
        self
    }

    fn settings(&self) -> MatcherSettings {
        MatcherSettings {
            pattern: self.pattern,
            mask: self.mask,
            op: self.op,
            trigger_at: self.trigger_at,
            predicate_mode: self.predicate_mode,
            range_min: self.range_min,
            range_max: self.range_max,
            set_values: self.set_values.clone(),
            match_count: self.match_count,
            holdoff_ns: self.holdoff_ns,
        }
    }

    fn configured_settings(
        mut settings: MatcherSettings,
        config: &NodeConfig,
    ) -> Result<MatcherSettings, ()> {
        for (key, value) in config {
            match (key.as_str(), value) {
                ("pattern", ConfigValue::U64(pattern)) => settings.pattern = *pattern,
                ("mask", ConfigValue::U64(mask)) => settings.mask = *mask,
                ("op", ConfigValue::Text(op)) => settings.op = MatchOp::parse(op).ok_or(())?,
                ("trigger_at", ConfigValue::Text(at)) => {
                    settings.trigger_at = TriggerAt::parse(at).ok_or(())?;
                }
                ("predicate", ConfigValue::Text(predicate)) => {
                    settings.predicate_mode = PredicateMode::parse(predicate).ok_or(())?;
                }
                ("range_min", ConfigValue::U64(minimum)) => settings.range_min = *minimum,
                ("range_max", ConfigValue::U64(maximum)) => settings.range_max = *maximum,
                ("set", ConfigValue::Text(values)) => {
                    settings.set_values = parse_set_values(values).ok_or(())?;
                }
                ("match_count", ConfigValue::U64(match_count)) if *match_count > 0 => {
                    settings.match_count = *match_count;
                }
                ("holdoff_ns", ConfigValue::U64(holdoff_ns)) => {
                    settings.holdoff_ns = *holdoff_ns;
                }
                _ => return Err(()),
            }
        }
        Ok(settings)
    }

    fn apply_settings(&mut self, settings: MatcherSettings) {
        self.pattern = settings.pattern;
        self.mask = settings.mask;
        self.op = settings.op;
        self.trigger_at = settings.trigger_at;
        self.predicate_mode = settings.predicate_mode;
        self.range_min = settings.range_min;
        self.range_max = settings.range_max;
        self.set_values = settings.set_values;
        self.match_count = settings.match_count.max(1);
        self.holdoff_ns = settings.holdoff_ns;
    }

    fn apply_scheduled_settings(&mut self, timestamp_ns: u64) {
        let mut due = Vec::new();
        {
            let mut scheduled = self
                .scheduled_settings
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while scheduled
                .front()
                .is_some_and(|(boundary, _)| boundary.timestamp_ns <= timestamp_ns)
            {
                due.push(scheduled.pop_front().expect("front was present").1);
            }
        }
        for config in due {
            let settings = Self::configured_settings(self.settings(), &config)
                .expect("scheduled matcher configuration was validated before enqueue");
            self.apply_settings(settings);
        }
    }

    fn predicate_matches(&self, value: u64) -> bool {
        let value = value & self.mask;
        match self.predicate_mode {
            PredicateMode::Compare => self.op.matches(value, self.pattern & self.mask),
            PredicateMode::InclusiveRange => {
                let minimum = self.range_min & self.mask;
                let maximum = self.range_max & self.mask;
                minimum <= maximum && (minimum..=maximum).contains(&value)
            }
            PredicateMode::Set => self
                .set_values
                .iter()
                .any(|candidate| candidate & self.mask == value),
        }
    }

    fn fill_heads(&mut self, inputs: &[InputPort]) -> WorkResult<()> {
        if self.word_head.is_none() && !self.word_eos {
            let mut receiver = inputs
                .first()
                .and_then(|port| port.get::<Word>(&mut self.input_buffer))
                .ok_or_else(|| WorkError::NodeError("Missing words input".to_owned()))?;
            match receiver.recv() {
                Ok(word) => self.word_head = Some(word),
                Err(WorkError::Shutdown) => self.word_eos = true,
                Err(error) => return Err(error),
            }
        }
        if self.manual_rearm && self.rearm_head.is_none() && !self.rearm_eos {
            let mut receiver = inputs
                .get(1)
                .and_then(|port| port.get::<Trigger>(&mut self.rearm_buffer))
                .ok_or_else(|| WorkError::NodeError("Missing rearm input".to_owned()))?;
            match receiver.recv() {
                Ok(rearm) => self.rearm_head = Some(rearm),
                Err(WorkError::Shutdown) => self.rearm_eos = true,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

fn parse_set_values(values: &str) -> Option<Vec<u64>> {
    values
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
                .map_or_else(|| value.parse(), |hex| u64::from_str_radix(hex, 16))
                .ok()
        })
        .collect()
}

impl ProcessNode for WordMatcher {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        2
    }

    fn num_outputs(&self) -> usize {
        3
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<Word>("words", 0, PortDirection::Input),
            PortSchema::new::<Trigger>("rearm", 1, PortDirection::Input),
        ]
    }

    /// Hot-appliable: `pattern` / `mask` (U64), `op`, and `trigger_at`.
    /// Takes effect for the next word; in-flight words already consumed
    /// keep the old match result (accepted hot-reconfigure semantics).
    fn apply_config(&mut self, config: &NodeConfig) -> ConfigOutcome {
        let Ok(settings) = Self::configured_settings(self.settings(), config) else {
            return ConfigOutcome::NeedsRestart;
        };
        self.apply_settings(settings);
        ConfigOutcome::Applied
    }

    fn configuration_scheduler(&self) -> Option<Arc<dyn ConfigurationScheduler>> {
        Some(Arc::new(WordMatcherConfigurationScheduler {
            scheduled: Arc::clone(&self.scheduled_settings),
        }))
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<Trigger>("trigger", 0, PortDirection::Output),
            PortSchema::state::<Sample>("matched", 1, PortDirection::Output),
            PortSchema::new::<Word>("matching_words", 2, PortDirection::Output)
                .with_default_buffer_capacity(8),
        ]
    }

    fn work_outcome(
        &mut self,
        inputs: &[InputPort],
        outputs: &[OutputPort],
    ) -> WorkResult<WorkOutcome> {
        self.work(inputs, outputs).map(WorkOutcome::progressed)
    }

    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        self.fill_heads(inputs)?;
        if self.word_head.is_none() && self.word_eos {
            return Err(WorkError::Shutdown);
        }

        let rearm_precedes_word = self.manual_rearm
            && self.rearm_head.is_some_and(|rearm| {
                self.word_head
                    .as_ref()
                    .is_none_or(|word| rearm.timestamp_ns <= word.timestamp_ns)
            });
        if rearm_precedes_word {
            self.rearm_head.take();
            self.armed = true;
            self.eligible_matches = 0;
            return Ok(0);
        }

        let trigger_out = outputs.first().and_then(|port| port.get::<Trigger>());
        // Optional visualization lane; None when unconnected.
        let pulse_out = outputs.get(1).and_then(|port| port.get::<Sample>());
        let word_out = outputs.get(2).and_then(|port| port.get::<Word>());

        // Level-stream contract: the pulse lane is a level, low at t=0.
        if !self.started {
            self.started = true;
            if let Some(pulse) = &pulse_out {
                pulse.send(Sample::new(false, 0))?;
            }
        }

        let word = self.word_head.take().expect("word head was selected");
        self.apply_scheduled_settings(word.timestamp_ns);
        let value = word.value;
        if self.predicate_matches(value) {
            let ts = match self.trigger_at {
                TriggerAt::Start => word.timestamp_ns,
                TriggerAt::End => word.end_ns(),
            };
            if ts < self.next_allowed_ns || (self.manual_rearm && !self.armed) {
                return Ok(0);
            }
            self.eligible_matches = self.eligible_matches.saturating_add(1);
            if self.eligible_matches < self.match_count {
                return Ok(0);
            }
            self.eligible_matches = 0;
            self.next_allowed_ns = ts.saturating_add(self.holdoff_ns);
            if self.manual_rearm {
                self.armed = false;
            }
            self.matches += 1;
            debug!(
                "[{}] match #{}: 0x{:06X} at {}ns",
                self.name, self.matches, value, ts
            );
            if let Some(trigger) = &trigger_out {
                trigger.send(Trigger::new(ts))?;
            }
            if let Some(pulse) = &pulse_out {
                if ts >= self.last_pulse_end {
                    pulse.send(Sample::new(true, ts))?;
                    pulse.send(Sample::new(false, ts + self.pulse_ns))?;
                    self.last_pulse_end = ts + self.pulse_ns;
                } else {
                    debug!(
                        "[{}] pulse at {}ns overlaps previous (ends {}ns), skipped",
                        self.name, ts, self.last_pulse_end
                    );
                }
            }
            if let Some(output) = &word_out {
                output.send(word)?;
            }
            return Ok(1);
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::bounded;
    use signal_runtime::{ChannelMessage, Sender, Watchdog};

    use super::*;

    fn run_to_shutdown(node: &mut dyn ProcessNode, inputs: &[InputPort], outputs: &[OutputPort]) {
        loop {
            match node.work(inputs, outputs) {
                Ok(_) => {}
                Err(WorkError::Shutdown) => break,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
    }

    #[test]
    fn matches_pattern_and_emits_triggers() {
        let wd = Watchdog::new();
        let (tx, rx) = bounded::<ChannelMessage<Word>>(16);
        let input = InputPort::new_with_watchdog(rx, &wd, "m", "words");
        let (ttx, trx) = bounded::<ChannelMessage<Trigger>>(16);
        let trigger_out =
            OutputPort::new_with_watchdog(Sender::new(vec![ttx]), &wd, "m", "trigger");
        let (ptx, prx) = bounded::<ChannelMessage<Sample>>(16);
        let pulse_out = OutputPort::new_with_watchdog(Sender::new(vec![ptx]), &wd, "m", "matched");

        tx.send(ChannelMessage::Sample(Word::new(0x600081, 100)))
            .unwrap();
        tx.send(ChannelMessage::Sample(Word::new(0x600000, 200)))
            .unwrap();
        tx.send(ChannelMessage::Sample(Word::new(0x600081, 300_000)))
            .unwrap();
        drop(tx);

        let mut m = WordMatcher::new(0x600081, 0xFFFFFF);
        run_to_shutdown(&mut m, &[input], &[trigger_out, pulse_out]);

        let triggers: Vec<Trigger> = trx
            .try_iter()
            .filter_map(|m| match m {
                ChannelMessage::Sample(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(triggers, vec![Trigger::new(100), Trigger::new(300_000)]);

        let pulses: Vec<Sample> = prx
            .try_iter()
            .filter_map(|m| match m {
                ChannelMessage::Sample(s) => Some(s),
                _ => None,
            })
            .collect();
        // initial low + two true/false pulse pairs
        assert_eq!(pulses[0], Sample::new(false, 0));
        assert_eq!(pulses[1], Sample::new(true, 100));
        assert_eq!(pulses[2], Sample::new(false, 1_100));
        assert_eq!(pulses[3], Sample::new(true, 300_000));
    }

    #[test]
    fn mask_limits_comparison() {
        let wd = Watchdog::new();
        let (tx, rx) = bounded::<ChannelMessage<Word>>(16);
        let input = InputPort::new_with_watchdog(rx, &wd, "m", "words");
        let (ttx, trx) = bounded::<ChannelMessage<Trigger>>(16);
        let trigger_out =
            OutputPort::new_with_watchdog(Sender::new(vec![ttx]), &wd, "m", "trigger");
        let (ptx, _prx) = bounded::<ChannelMessage<Sample>>(16);
        let pulse_out = OutputPort::new_with_watchdog(Sender::new(vec![ptx]), &wd, "m", "matched");

        // Match on register byte only (0x60xxxx)
        tx.send(ChannelMessage::Sample(Word::new(0x600081, 1)))
            .unwrap();
        tx.send(ChannelMessage::Sample(Word::new(0x600000, 2)))
            .unwrap();
        tx.send(ChannelMessage::Sample(Word::new(0x6A0000, 3)))
            .unwrap();
        drop(tx);

        let mut m = WordMatcher::new(0x600000, 0xFF0000);
        run_to_shutdown(&mut m, &[input], &[trigger_out, pulse_out]);

        let triggers: Vec<u64> = trx
            .try_iter()
            .filter_map(|m| match m {
                ChannelMessage::Sample(t) => Some(t.timestamp_ns),
                _ => None,
            })
            .collect();
        assert_eq!(triggers, vec![1, 2]);
    }

    #[test]
    fn parallel_words_match_on_value() {
        let wd = Watchdog::new();
        let (tx, rx) = bounded::<ChannelMessage<Word>>(16);
        let input = InputPort::new_with_watchdog(rx, &wd, "m", "words");
        let (ttx, trx) = bounded::<ChannelMessage<Trigger>>(16);
        let trigger_out =
            OutputPort::new_with_watchdog(Sender::new(vec![ttx]), &wd, "m", "trigger");
        let (ptx, _prx) = bounded::<ChannelMessage<Sample>>(16);
        let pulse_out = OutputPort::new_with_watchdog(Sender::new(vec![ptx]), &wd, "m", "matched");

        for (v, ts) in [(0xAAu64, 10u64), (0x55, 20), (0xAA, 30)] {
            tx.send(ChannelMessage::Sample(Word::new(v, ts))).unwrap();
        }
        drop(tx);

        let mut m = WordMatcher::new(0xAA, 0xFF);
        run_to_shutdown(&mut m, &[input], &[trigger_out, pulse_out]);

        let triggers: Vec<u64> = trx
            .try_iter()
            .filter_map(|m| match m {
                ChannelMessage::Sample(t) => Some(t.timestamp_ns),
                _ => None,
            })
            .collect();
        assert_eq!(triggers, vec![10, 30]);
    }

    /// The trigger lands at the word's end by default (where the command
    /// has fully arrived), or at its start when configured — for
    /// instantaneous words (duration 0) the two coincide.
    #[test]
    fn trigger_lands_at_word_end_by_default_and_start_when_configured() {
        let run = |matcher: WordMatcher| -> Vec<u64> {
            let wd = Watchdog::new();
            let (tx, rx) = bounded::<ChannelMessage<Word>>(16);
            let input = InputPort::new_with_watchdog(rx, &wd, "m", "words");
            let (ttx, trx) = bounded::<ChannelMessage<Trigger>>(16);
            let trigger_out =
                OutputPort::new_with_watchdog(Sender::new(vec![ttx]), &wd, "m", "trigger");
            let (ptx, _prx) = bounded::<ChannelMessage<Sample>>(16);
            let pulse_out =
                OutputPort::new_with_watchdog(Sender::new(vec![ptx]), &wd, "m", "matched");

            // A 24-bit-word-like span (start 100, last edge 2_400) and an
            // instantaneous word.
            tx.send(ChannelMessage::Sample(Word::spanning(0xAA, 100, 2_300)))
                .unwrap();
            tx.send(ChannelMessage::Sample(Word::new(0xAA, 10_000)))
                .unwrap();
            drop(tx);

            let mut m = matcher;
            run_to_shutdown(&mut m, &[input], &[trigger_out, pulse_out]);
            trx.try_iter()
                .filter_map(|m| match m {
                    ChannelMessage::Sample(t) => Some(t.timestamp_ns),
                    _ => None,
                })
                .collect()
        };

        assert_eq!(run(WordMatcher::new(0xAA, 0xFF)), vec![2_400, 10_000]);
        assert_eq!(
            run(WordMatcher::new(0xAA, 0xFF).with_trigger_at(TriggerAt::Start)),
            vec![100, 10_000]
        );
    }

    #[test]
    fn inequality_ops_compare_masked_values() {
        let words: Vec<(u64, u64)> = vec![(0x10, 1), (0x20, 2), (0x30, 3)];
        let run_with_op = |op: MatchOp| -> Vec<u64> {
            let wd = Watchdog::new();
            let (tx, rx) = bounded::<ChannelMessage<Word>>(16);
            let input = InputPort::new_with_watchdog(rx, &wd, "m", "words");
            let (ttx, trx) = bounded::<ChannelMessage<Trigger>>(16);
            let trigger_out =
                OutputPort::new_with_watchdog(Sender::new(vec![ttx]), &wd, "m", "trigger");
            let (ptx, _prx) = bounded::<ChannelMessage<Sample>>(16);
            let pulse_out =
                OutputPort::new_with_watchdog(Sender::new(vec![ptx]), &wd, "m", "matched");
            for (v, ts) in &words {
                tx.send(ChannelMessage::Sample(Word::new(*v, *ts))).unwrap();
            }
            drop(tx);
            let mut m = WordMatcher::new(0x20, u64::MAX).with_op(op);
            run_to_shutdown(&mut m, &[input], &[trigger_out, pulse_out]);
            trx.try_iter()
                .filter_map(|m| match m {
                    ChannelMessage::Sample(t) => Some(t.timestamp_ns),
                    _ => None,
                })
                .collect()
        };

        assert_eq!(run_with_op(MatchOp::Eq), vec![2]);
        assert_eq!(run_with_op(MatchOp::Ne), vec![1, 3]);
        assert_eq!(run_with_op(MatchOp::Lt), vec![1]);
        assert_eq!(run_with_op(MatchOp::Le), vec![1, 2]);
        assert_eq!(run_with_op(MatchOp::Gt), vec![3]);
        assert_eq!(run_with_op(MatchOp::Ge), vec![2, 3]);
    }

    #[test]
    fn scheduled_pattern_uses_old_config_before_boundary_and_new_at_boundary() {
        let wd = Watchdog::new();
        let (tx, rx) = bounded::<ChannelMessage<Word>>(16);
        let input = InputPort::new_with_watchdog(rx, &wd, "m", "words");
        let (ttx, trx) = bounded::<ChannelMessage<Trigger>>(16);
        let trigger_out =
            OutputPort::new_with_watchdog(Sender::new(vec![ttx]), &wd, "m", "trigger");
        let (ptx, _prx) = bounded::<ChannelMessage<Sample>>(16);
        let pulse_out = OutputPort::new_with_watchdog(Sender::new(vec![ptx]), &wd, "m", "matched");
        for (value, timestamp) in [(1, 100), (2, 199), (2, 200), (1, 201)] {
            tx.send(ChannelMessage::Sample(Word::new(value, timestamp)))
                .unwrap();
        }
        drop(tx);

        let mut matcher = WordMatcher::new(1, u64::MAX);
        let config = NodeConfig::from([("pattern".to_owned(), ConfigValue::U64(2))]);
        assert_eq!(
            matcher
                .configuration_scheduler()
                .unwrap()
                .schedule_config(&config, ConfigurationBoundary::new(2, 200)),
            ConfigOutcome::Applied
        );
        run_to_shutdown(&mut matcher, &[input], &[trigger_out, pulse_out]);

        let timestamps: Vec<_> = trx
            .try_iter()
            .filter_map(|message| match message {
                ChannelMessage::Sample(trigger) => Some(trigger.timestamp_ns),
                _ => None,
            })
            .collect();
        assert_eq!(timestamps, vec![100, 200]);
    }

    fn run_extended(
        mut matcher: WordMatcher,
        words: &[(u64, u64)],
        rearms: &[u64],
    ) -> (Vec<u64>, Vec<u64>) {
        let watchdog = Watchdog::new();
        let (word_tx, word_rx) = bounded::<ChannelMessage<Word>>(32);
        for (value, timestamp_ns) in words {
            word_tx
                .send(ChannelMessage::Sample(Word::new(*value, *timestamp_ns)))
                .unwrap();
        }
        drop(word_tx);
        let (rearm_tx, rearm_rx) = bounded::<ChannelMessage<Trigger>>(32);
        for timestamp_ns in rearms {
            rearm_tx
                .send(ChannelMessage::Sample(Trigger::new(*timestamp_ns)))
                .unwrap();
        }
        drop(rearm_tx);
        let inputs = [
            InputPort::new_with_watchdog(word_rx, &watchdog, "matcher", "words"),
            InputPort::new_with_watchdog(rearm_rx, &watchdog, "matcher", "rearm"),
        ];
        let (trigger_tx, trigger_rx) = bounded::<ChannelMessage<Trigger>>(32);
        let (pulse_tx, _pulse_rx) = bounded::<ChannelMessage<Sample>>(32);
        let (matching_word_tx, matching_word_rx) = bounded::<ChannelMessage<Word>>(32);
        let outputs = [
            OutputPort::new_with_watchdog(
                Sender::new(vec![trigger_tx]),
                &watchdog,
                "matcher",
                "trigger",
            ),
            OutputPort::new_with_watchdog(
                Sender::new(vec![pulse_tx]),
                &watchdog,
                "matcher",
                "matched",
            ),
            OutputPort::new_with_watchdog(
                Sender::new(vec![matching_word_tx]),
                &watchdog,
                "matcher",
                "matching_words",
            ),
        ];
        run_to_shutdown(&mut matcher, &inputs, &outputs);
        let triggers = trigger_rx
            .try_iter()
            .filter_map(|message| match message {
                ChannelMessage::Sample(trigger) => Some(trigger.timestamp_ns),
                _ => None,
            })
            .collect();
        let matching_words = matching_word_rx
            .try_iter()
            .filter_map(|message| match message {
                ChannelMessage::Sample(word) => Some(word.value),
                _ => None,
            })
            .collect();
        (triggers, matching_words)
    }

    #[test]
    fn range_and_set_predicates_emit_the_matching_words() {
        let words = [(0x10, 1), (0x20, 2), (0x30, 3), (0x40, 4), (0x50, 5)];
        assert_eq!(
            run_extended(
                WordMatcher::new(0, u64::MAX).with_inclusive_range(0x20, 0x40),
                &words,
                &[],
            ),
            (vec![2, 3, 4], vec![0x20, 0x30, 0x40])
        );
        assert_eq!(
            run_extended(
                WordMatcher::new(0, u64::MAX).with_set(vec![0x10, 0x50]),
                &words,
                &[],
            ),
            (vec![1, 5], vec![0x10, 0x50])
        );
    }

    #[test]
    fn match_count_and_holdoff_apply_before_outputs() {
        let matcher = WordMatcher::new(1, u64::MAX)
            .with_match_count(2)
            .with_holdoff_ns(25);
        assert_eq!(
            run_extended(matcher, &[(1, 10), (1, 20), (1, 30), (1, 50), (1, 80)], &[],),
            (vec![20, 80], vec![1, 1])
        );
    }

    #[test]
    fn explicit_rearm_allows_the_next_matching_word() {
        let matcher = WordMatcher::new(1, u64::MAX).with_manual_rearm(true);
        assert_eq!(
            run_extended(matcher, &[(1, 10), (1, 20), (1, 30), (1, 40)], &[25]),
            (vec![10, 30], vec![1, 1])
        );
    }
}
