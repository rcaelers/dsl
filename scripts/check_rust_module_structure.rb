#!/usr/bin/env ruby
# frozen_string_literal: true

# Enforces docs/aspects/responsibility_visibility.md. This deliberately
# checks source structure rather than formatting; rustc remains authoritative
# for name resolution and `unreachable_pub`.

ROOT = File.expand_path("..", __dir__)
SOURCE_GLOBS = ["crates/**/*.rs", "plugins/**/*.rs", "tests/**/*.rs", "benches/**/*.rs"].freeze
ROOT_FILES = %w[lib.rs main.rs mod.rs].freeze
DECLARATIVE_SELECTION_FACADES = %w[
  crates/platform/src/host/mod.rs
  crates/logic_analyzer_device_dslogic/src/device/mod.rs
  crates/logic_analyzer_device_dslogic/src/device/dslogic_u3pro16/mod.rs
].freeze

PUBLIC_MODULES = {
  "crates/signal_derived/src/lib.rs" => %w[derived_word_store],
  "crates/signal_capture_session/src/lib.rs" => %w[live_capture live_capture_store],
  "crates/logic_analyzer_capture_formats/src/lib.rs" => %w[dsl_file sigrok_file],
  "crates/logic_analyzer_protocol_decoders/src/lib.rs" => %w[
    i2c_decoder packet_framer parallel_decoder sigrok_decoder spi_decoder types uart_decoder
  ],
  "crates/signal_transforms/src/lib.rs" => %w[
    buffer edge_detector event_control event_counter event_gate logic_gate sr_latch text_formatter
    timeline_marker word_field_extractor word_matcher
  ],
  "crates/signal_sinks/src/lib.rs" => %w[
    binary_file_writer csv_word_writer discard_writer text_file_writer tgck_recorder
  ],
  "crates/signal_generators/src/lib.rs" => %w[synthetic_capture_source synthetic_uart_source],
  "crates/logic_analyzer_graph_capabilities/src/lib.rs" => %w[node node_support],
  "crates/widgets/node_graph/src/lib.rs" => %w[api]
}.freeze

REQUIRED_PRIVATE_OWNER_MODULES = {
  "crates/widgets/node_graph/src/widget/graph/mod.rs" => %w[
    input_dispatch interaction interaction_state response selection wire
  ],
  "crates/widgets/panel_layout/src/lib.rs" => %w[
    contract controls geometry icon layout tree
  ],
  "crates/widgets/trigger_editor/src/lib.rs" => %w[
    contract model presentation widget
  ]
}.freeze

errors = []

capture_source_metadata_path = File.join(
  ROOT,
  "crates/signal_capture_session/src/capture_source_metadata.rs"
)
capture_source_metadata = File.read(capture_source_metadata_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
%w[presentation channel_names configured_acquisition].each do |operation|
  signature = capture_source_metadata[/fn\s+#{operation}\b(?<signature>.*?)(?:\{|;)/m, :signature]
  next if signature&.include?("CaptureSourceMetadataError") &&
          !signature.match?(/,\s*String\s*>\s*$/m)

  errors << "crates/signal_capture_session/src/capture_source_metadata.rs: #{operation} must retain CaptureSourceMetadataError"
end
capture_source_metadata_error = capture_source_metadata[
  /pub enum CaptureSourceMetadataError\s*\{(?<body>.*?)^\}/m,
  :body
].to_s
%w[Access Decode Acquisition].each do |variant|
  next if capture_source_metadata_error.match?(
    /^\s*#{variant}\(\#\[source\]\s*Arc<dyn Error \+ Send \+ Sync>\),$/
  )

  errors << "crates/signal_capture_session/src/capture_source_metadata.rs: CaptureSourceMetadataError must retain its #{variant.downcase} source"
end

{
  "crates/logic_analyzer_capture_formats/src/dsl_file/prepared_file.rs" => %w[access decode],
  "crates/logic_analyzer_capture_formats/src/sigrok_file/prepared_file.rs" => %w[access decode],
  "crates/logic_analyzer_device_dslogic/src/device/dslogic_u3pro16/host_factory.rs" => %w[acquisition]
}.each do |path, categories|
  source = File.read(File.join(ROOT, path))
  categories.each do |category|
    next if source.include?("CaptureSourceMetadataError::#{category}")

    errors << "#{path}: capture-source metadata adapter must preserve its #{category} cause"
  end
end

capture_feature_contract_path = File.join(
  ROOT,
  "crates/logic_analyzer_graph_capabilities/src/node/contracts.rs"
)
capture_feature_contract = File.read(capture_feature_contract_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
capture_presentation_signature = capture_feature_contract[
  /fn\s+capture_presentation\b(?<signature>.*?)(?:\{|;)/m,
  :signature
].to_s
unless capture_presentation_signature.include?("CaptureSourceFeatureError")
  errors << "crates/logic_analyzer_graph_capabilities/src/node/contracts.rs: capture_presentation must retain CaptureSourceFeatureError"
end
unless capture_feature_contract.match?(
  /^\s*Metadata\(\#\[from\]\s*CaptureSourceMetadataError\),$/
)
  errors << "crates/logic_analyzer_graph_capabilities/src/node/contracts.rs: CaptureSourceFeatureError must retain CaptureSourceMetadataError"
end
unless capture_feature_contract.match?(/pub struct CaptureGraphSourceError\s*\{/) &&
       capture_feature_contract.match?(
         /source:\s*Box<dyn StdError \+ Send \+ Sync>/
       ) && capture_feature_contract.match?(
         /fn\s+create\b.*?Result<Box<dyn ProcessNode>,\s*CaptureGraphSourceError>/m
       )
  errors << "crates/logic_analyzer_graph_capabilities/src/node/contracts.rs: capture graph-source construction must retain typed causes"
end

persisted_state_contract = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_capabilities/src/node_support/state.rs"
))
timeline_feature_error = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_capabilities/src/node/error.rs"
))
unless persisted_state_contract.match?(/pub enum PersistedStateError\s*\{/) &&
       persisted_state_contract.match?(/^\s*Decode\(\#\[source\]\s*serde_json::Error\),$/) &&
       persisted_state_contract.match?(/^\s*Encode\(\#\[source\]\s*serde_json::Error\),$/) &&
       persisted_state_contract.match?(
         /pub fn parse_state\b.*?Result<T, PersistedStateError>/m
       )
  errors << "crates/logic_analyzer_graph_capabilities/src/node_support/state.rs: persisted state must retain JSON codec causes"
end
unless timeline_feature_error.match?(/pub enum TimelineFeatureError\s*\{/) &&
       timeline_feature_error.match?(/^\s*State\(\#\[from\]\s*PersistedStateError\),$/) &&
       capture_feature_contract.scan(/TimelineFeatureError/).length >= 4
  errors << "crates/logic_analyzer_graph_capabilities/src/node: timeline features must retain typed state and edit failures"
end
unless timeline_feature_error.match?(/pub enum LiveCaptureFeatureError\s*\{/) &&
       timeline_feature_error.match?(/^\s*State\(\#\[from\]\s*PersistedStateError\),$/) &&
       timeline_feature_error.match?(
         /^\s*Metadata\(\#\[from\]\s*CaptureSourceMetadataError\),$/
       ) && capture_feature_contract.scan(/LiveCaptureFeatureError/).length >= 4
  errors << "crates/logic_analyzer_graph_capabilities/src/node: live-capture features must retain typed state and metadata failures"
end

timeline_compiler_error = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_compiler/src/error.rs"
))
timeline_compiler = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_compiler/src/graph.rs"
))
unless timeline_compiler_error.match?(/pub enum TimelineOperationError\s*\{/) &&
       timeline_compiler_error.match?(
         /Feature\s*\{.*?source:\s*TimelineFeatureError,/m
       ) && timeline_compiler_error.match?(
         /Self::Feature\s*\{\s*source,\s*\.\.\s*\}\s*=>\s*Some\(source\)/m
       ) && timeline_compiler.scan(/TimelineOperationError::feature/).length >= 4
  errors << "crates/logic_analyzer_graph_compiler: timeline operations must retain graph-feature failures"
end
unless timeline_compiler_error.match?(/pub enum LiveCaptureOperationError\s*\{/) &&
       timeline_compiler_error.match?(
         /Feature\s*\{.*?source:\s*LiveCaptureFeatureError,/m
       ) && timeline_compiler_error.match?(
         /Self::Feature\s*\{\s*source,\s*\.\.\s*\}\s*=>\s*Some\(source\)/m
       ) && timeline_compiler.scan(/LiveCaptureOperationError::feature/).length >= 3
  errors << "crates/logic_analyzer_graph_compiler: live-capture operations must retain graph-feature failures"
end

capture_validation = File.read(File.join(
  ROOT,
  "crates/signal_capture_session/src/live_capture/validation.rs"
))
capture_implementation = File.read(File.join(
  ROOT,
  "crates/signal_capture_session/src/live_capture/implementation.rs"
))
capture_analysis = File.read(File.join(
  ROOT,
  "crates/signal_capture_session/src/live_capture/analysis.rs"
))
capture_acquisition = File.read(File.join(
  ROOT,
  "crates/signal_capture_session/src/live_capture/acquisition.rs"
))
unless capture_validation.match?(/pub enum CaptureValidationError\s*\{/) &&
       capture_implementation.scan(/Result<Self,\s*CaptureValidationError>/).length >= 2 &&
       capture_analysis.match?(/Result<Self,\s*CaptureValidationError>/)
  errors << "crates/signal_capture_session/src/live_capture: constructor validation must use CaptureValidationError"
end
unless capture_acquisition.match?(
  /InvalidRequest\(\#\[source\]\s*Box<dyn StdError \+ Send \+ Sync>\)/
) && capture_acquisition.match?(/pub fn invalid_request\b/)
  errors << "crates/signal_capture_session/src/live_capture/acquisition.rs: invalid acquisition requests must retain typed causes"
end

capture_discovery_plan = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_plan/src/plan/types.rs"
)).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
unless capture_discovery_plan.match?(
  /SourceFeature\s*\{.*?\#\[source\]\s*error:\s*CaptureSourceFeatureError,/m
)
  errors << "crates/logic_analyzer_graph_plan/src/plan/types.rs: capture discovery must retain its typed feature cause"
end
unless capture_discovery_plan.match?(
  /Identity\(\#\[source\]\s*Arc<serde_json::Error>\),/
)
  errors << "crates/logic_analyzer_graph_plan/src/plan/types.rs: capture discovery must retain its identity-encoding cause"
end

capture_discovery = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_compiler/src/graph.rs"
))
unless capture_discovery.match?(
  /fn\s+discover_capture_presentation_with_subscriptions\b.*?CapturePresentationDiscoveryError/m
) && capture_discovery.include?("CapturePresentationDiscoveryError::source_feature")
  errors << "crates/logic_analyzer_graph_compiler/src/graph.rs: capture discovery must preserve typed graph-feature failures"
end

sigrok_runtime_path = File.join(
  ROOT,
  "crates/logic_analyzer_protocol_decoders/src/sigrok_decoder/runtime.rs"
)
sigrok_runtime_source = File.read(sigrok_runtime_path)
sigrok_runtime_contracts = {
  "Sigrok decoder discovery" =>
    /fn\s+discover\b.*?Result<SigrokDecoderDescriptor,\s*SigrokDecoderRuntimeError>/m,
  "Sigrok decoder creation" =>
    /fn\s+create\b.*?Result<Box<dyn ProcessNode>,\s*SigrokDecoderRuntimeError>/m,
  "Sigrok catalog scanning" =>
    /fn\s+scan\b.*?Result<SigrokCatalogSnapshot,\s*SigrokCatalogError>/m
}.freeze
sigrok_runtime_contracts.each do |contract, pattern|
  next if sigrok_runtime_source.match?(pattern)

  errors << "crates/logic_analyzer_protocol_decoders/src/sigrok_decoder/runtime.rs: #{contract} must retain its owner-typed error contract"
end
sigrok_decoder_error = sigrok_runtime_source[/pub enum SigrokDecoderRuntimeError\s*\{(?<body>.*?)^\}/m, :body].to_s
sigrok_decoder_error_contracts = {
  "discovery" => /Discovery\(\#\[source\]\s*SigrokDecoderDiscoveryError\),/,
  "configuration" => /Configuration\(String\),/,
  "execution startup" => /ExecutionStart\(\#\[source\]\s*SigrokExecutionStartError\),/
}.freeze
sigrok_decoder_error_contracts.each do |failure, pattern|
  next if sigrok_decoder_error.match?(pattern)

  errors << "crates/logic_analyzer_protocol_decoders/src/sigrok_decoder/runtime.rs: SigrokDecoderRuntimeError must classify #{failure} failures"
end
sigrok_discovery_error_path = File.join(
  ROOT,
  "crates/logic_analyzer_protocol_decoders/src/sigrok_decoder/discovery_error.rs"
)
sigrok_discovery_error_source = File.read(sigrok_discovery_error_path)
unless sigrok_discovery_error_source.match?(
  /pub enum SigrokDecoderDiscoveryError\s*\{.*?Inspection\s*\{.*?\#\[source\].*?Fingerprint\s*\{.*?\#\[source\]/m
)
  errors << "crates/logic_analyzer_protocol_decoders/src/sigrok_decoder/discovery_error.rs: decoder discovery must retain typed inspection and fingerprint sources"
end
unless sigrok_discovery_error_source.match?(
  /pub enum SigrokCatalogError\s*\{.*?Scan\s*\{.*?\#\[source\]/m
)
  errors << "crates/logic_analyzer_protocol_decoders/src/sigrok_decoder/discovery_error.rs: fatal catalog scanning must retain its host source"
end

trigger_program_path = File.join(ROOT, "crates/logic_analyzer_trigger/src/program.rs")
trigger_program_source = File.read(trigger_program_path)
unless trigger_program_source.scan(/Result<Self,\s*TriggerSchemaError>/).length >= 6 &&
       trigger_program_source.match?(/pub fn simple_program\b.*?Result<Option<TriggerProgram>,\s*TriggerSchemaError>/m)
  errors << "crates/logic_analyzer_trigger/src/program.rs: public trigger construction must use its typed schema or validation errors"
end
unless trigger_program_source.match?(
  /pub enum TriggerProgramEditError\s*\{.*?Validation\(.*?TriggerValidationErrors,?\s*\).*?Schema\(.*?TriggerSchemaError,?\s*\)/m
)
  errors << "crates/logic_analyzer_trigger/src/program.rs: trigger edits must retain validation and schema causes"
end

trigger_editor_model_path = File.join(ROOT, "crates/widgets/trigger_editor/src/model.rs")
trigger_editor_model_source = File.read(trigger_editor_model_path)
unless trigger_editor_model_source.match?(
  /pub fn apply\b.*?Result<Option<TriggerProgram>,\s*TriggerEditorError>/m
)
  errors << "crates/widgets/trigger_editor/src/model.rs: trigger reducer must expose its widget-owned typed error"
end

trigger_configuration_path = File.join(
  ROOT,
  "crates/logic_analyzer_graph_capabilities/src/node_support/contracts.rs"
)
trigger_configuration_source = File.read(trigger_configuration_path)
unless trigger_configuration_source.match?(
  /impl TriggerConfigurationFeature\s*\{.*?pub fn new\b.*?Result<Self,\s*TriggerConfigurationError>/m
)
  errors << "crates/logic_analyzer_graph_capabilities/src/node_support/contracts.rs: trigger configuration assembly must expose its owner-typed error"
end

node_graph_widget_path = File.join(
  ROOT,
  "crates/widgets/node_graph/src/widget/graph/widget.rs"
)
node_graph_widget_source = File.read(node_graph_widget_path)
unless node_graph_widget_source.match?(
  /pub fn snapshot_value\b.*?Result<serde_json::Value,\s*GraphSnapshotError>/m
)
  errors << "crates/widgets/node_graph/src/widget/graph/widget.rs: document snapshots must retain their typed JSON serialization failure"
end

source_preparation_contract_path = File.join(
  ROOT,
  "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs"
)
source_preparation_contract = File.read(source_preparation_contract_path)
unless source_preparation_contract.match?(
  /^\s*Discovery\(\#\[source\]\s*CapturePresentationDiscoveryError\),$/
)
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs: source preparation must retain typed capture-discovery failures"
end
%w[Metadata Index].each do |variant|
  next if source_preparation_contract.match?(
    /^\s*#{variant}\(\#\[source\]\s*Arc<signal_capture::Error>\),$/
  )

  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs: SourcePreparationError must retain its #{variant.downcase} capture source"
end
unless source_preparation_contract.match?(
  /^\s*Executor\(\#\[source\]\s*WorkExecutorError\),$/
)
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs: SourcePreparationError must retain the typed host-work executor failure"
end
unless source_preparation_contract.match?(
  /^\s*WorkerProtocol\(\#\[source\]\s*SourcePreparationProtocolError\),$/
)
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs: SourcePreparationError must retain the typed preparation-protocol failure"
end
unless source_preparation_contract.match?(
  /pub enum SourcePreparationProtocolError\s*\{.*?UnexpectedResponse\s*\{/m
)
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs: preparation protocol failures must classify unexpected worker responses"
end
source_preparation_implementation = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation.rs"
)).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
%w[metadata index].each do |category|
  next if source_preparation_implementation.include?("SourcePreparationError::#{category}")

  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation.rs: capture #{category} failures must cross the typed source-preparation boundary"
end
if source_preparation_contract.scan(/Failed\(SourcePreparationError\)/).length < 2
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs: source-preparation updates and status must retain their typed failure cause"
end
source_preparation_executor = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_executor.rs"
))
unless source_preparation_executor.match?(
  /pub type SourcePreparationResult\s*=\s*Result<PreparedCaptureData, SourcePreparationError>/
)
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_executor.rs: preparation tasks must retain SourcePreparationError"
end
if source_preparation_executor.match?(
  /SourcePreparationError::(?:Executor|WorkerProtocol)\(\s*(?:format!|"[^"]*")/
)
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_executor.rs: executor and worker-protocol failures must not be flattened into strings"
end

derived_cache_policy_path = File.join(
  ROOT,
  "crates/logic_analyzer_graph_runtime/src/runtime/cache_policy.rs"
)
derived_cache_policy = File.read(derived_cache_policy_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
%w[Store Executor].each do |variant|
  lower_error = variant == "Store" ? "StoreError" : "WorkExecutorError"
  next if derived_cache_policy.match?(
    /^\s*#{variant}\(\#\[from\]\s*#{lower_error}\),$/
  )

  errors << "crates/logic_analyzer_graph_runtime/src/runtime/cache_policy.rs: DerivedCacheError must retain its #{variant.downcase} source"
end
unless derived_cache_policy.match?(
  /Option<Result<DerivedCacheClearStats, DerivedCacheError>>/
)
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/cache_policy.rs: asynchronous cache cleanup must retain DerivedCacheError"
end
if derived_cache_policy.match?(/Result<[^\n]*,\s*String>/) ||
   derived_cache_policy.match?(/map_err\s*\(\s*\|[^|]*\|[^\n]*\.to_string\(\)/)
  errors << "crates/logic_analyzer_graph_runtime/src/runtime/cache_policy.rs: cache failures must not collapse into display strings"
end

graph_runtime_service = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_runtime/src/runtime/service.rs"
))
%w[
  clear_derived_cache_entry start_clear_derived_caches clear_derived_caches
  inspect_derived_cache_entry
].each do |operation|
  next if graph_runtime_service.match?(
    /pub fn\s+#{operation}\b[^\{]*->\s*Result<[^\{]*DerivedCacheError>\s*\{/m
  )

  errors << "crates/logic_analyzer_graph_runtime/src/runtime/service.rs: #{operation} must retain DerivedCacheError"
end

capture_worker_protocol_path = File.join(
  ROOT,
  "crates/signal_capture/src/capture/host_protocol.rs"
)
capture_worker_protocol = File.read(capture_worker_protocol_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
unless capture_worker_protocol.match?(/Failed\s*\{.*?error:\s*CaptureWorkerFailure,/m)
  errors << "crates/signal_capture/src/capture/host_protocol.rs: capture-worker terminal diagnostics must retain CaptureWorkerFailure"
end

capture_worker_runtime_path = File.join(
  ROOT,
  "crates/signal_capture/src/capture/worker_runtime.rs"
)
capture_worker_runtime = File.read(capture_worker_runtime_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
unless capture_worker_runtime.match?(
  /pub fn\s+register\b.*?Result<\(\),\s*CaptureWorkerOperationRegistrationError>/m
)
  errors << "crates/signal_capture/src/capture/worker_runtime.rs: capture operation registration must retain its typed error"
end
unless capture_worker_runtime.match?(
  /fn\s+prepare\b.*?Result<CaptureWorkerPreparedIndex,\s*CaptureWorkerOperationPreparationError>/m
)
  errors << "crates/signal_capture/src/capture/worker_runtime.rs: capture operation preparation must retain its typed error"
end
if capture_worker_runtime.match?(/Result<.*?,\s*String>/m)
  errors << "crates/signal_capture/src/capture/worker_runtime.rs: capture operation failures must not collapse into display strings"
end
%w[
  encode_capture_worker_request decode_capture_worker_request
  encode_capture_worker_messages decode_capture_worker_messages
].each do |operation|
  next if capture_worker_protocol.match?(
    /pub fn\s+#{operation}\b[^\{]*->\s*Result<[^\{]*,\s*CaptureWorkerCodecError>\s*\{/m
  )

  errors << "crates/signal_capture/src/capture/host_protocol.rs: #{operation} must retain CaptureWorkerCodecError"
end
if capture_worker_protocol.match?(/Result<.*?,\s*String>/m)
  errors << "crates/signal_capture/src/capture/host_protocol.rs: codec failures must not collapse into display strings"
end

capture_worker_client_path = File.join(
  ROOT,
  "crates/signal_capture/src/capture/worker_client.rs"
)
capture_worker_client = File.read(capture_worker_client_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
%w[new submit_preparation submit_query submit_replay publish].each do |operation|
  next if capture_worker_client.match?(
    /pub fn\s+#{operation}\b[^\{]*->\s*Result<[^\{]*,\s*CaptureWorkerClientError>\s*\{/m
  )

  errors << "crates/signal_capture/src/capture/worker_client.rs: #{operation} must retain CaptureWorkerClientError"
end
unless capture_worker_client.match?(
  /pub fn\s+fail_all\b.*?error:\s*CaptureWorkerTransportFailure/m
)
  errors << "crates/signal_capture/src/capture/worker_client.rs: disconnects must retain CaptureWorkerTransportFailure"
end

capture_query_path = File.join(ROOT, "crates/signal_capture/src/capture/query.rs")
capture_query = File.read(capture_query_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
unless capture_query.match?(
  /Complete\(std::result::Result<CaptureSampledWindow, CaptureIndexQueryError>\)/
)
  errors << "crates/signal_capture/src/capture/query.rs: query updates must retain CaptureIndexQueryError"
end
unless capture_query.match?(
  /fn\s+submit\b[^\{]*->\s*std::result::Result<u64, CaptureIndexQueryError>/m
)
  errors << "crates/signal_capture/src/capture/query.rs: query submission must retain CaptureIndexQueryError"
end
if capture_query.match?(/Result<[^>\n]*,\s*String>/)
  errors << "crates/signal_capture/src/capture/query.rs: query failures must not collapse into display strings"
end

capture_errors = File.read(File.join(ROOT, "crates/signal_capture/src/errors.rs"))
unless capture_errors.match?(
  /^\s*CaptureQuery\(\#\[source\]\s*CaptureIndexQueryError\),$/
)
  errors << "crates/signal_capture/src/errors.rs: capture query failures must retain their typed source"
end

unless capture_worker_client.include?(
  "CaptureIndexQueryError::Submission(Box::new(error))"
)
  errors << "crates/signal_capture/src/capture/worker_client.rs: query submission must preserve its capture-worker client source"
end
unless capture_worker_client.include?(
  "CaptureIndexQueryError::Execution(Box::new(error))"
)
  errors << "crates/signal_capture/src/capture/worker_client.rs: query execution must preserve its capture-worker failure source"
end

%w[WorkerClient Worker].each do |variant|
  next if source_preparation_contract.match?(
    /^\s*#{variant}\(\#\[source\]\s*CaptureWorker(?:ClientError|Failure)\),$/
  )

  errors << "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs: SourcePreparationError must retain capture-worker #{variant.downcase} failures"
end

graph_worker_contract = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_graph_orchestration/src/worker_execution.rs"
))
unless graph_worker_contract.match?(/Failed\s*\{.*?error:\s*GraphWorkerFailure,/m)
  errors << "crates/logic_analyzer_graph_orchestration/src/worker_execution.rs: graph-worker terminal diagnostics must retain GraphWorkerFailure"
end
unless graph_worker_contract.match?(
  /Transport\(\#\[source\]\s*GraphWorkerTransportFailure\)/
)
  errors << "crates/logic_analyzer_graph_orchestration/src/worker_execution.rs: graph-worker transport failures must retain their typed cause"
end

graph_worker_codec_path = File.join(
  ROOT,
  "crates/logic_analyzer_graph_orchestration/src/worker_execution_codec.rs"
)
graph_worker_codec = File.read(graph_worker_codec_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
%w[
  encode_graph_worker_request decode_graph_worker_request
  encode_graph_worker_messages decode_graph_worker_messages
].each do |operation|
  next if graph_worker_codec.match?(
    /pub fn\s+#{operation}\b[^\{]*->\s*Result<[^\{]*,\s*GraphWorkerCodecError>\s*\{/m
  )

  errors << "crates/logic_analyzer_graph_orchestration/src/worker_execution_codec.rs: #{operation} must retain GraphWorkerCodecError"
end
if graph_worker_codec.match?(/Result<.*?,\s*String>/m)
  errors << "crates/logic_analyzer_graph_orchestration/src/worker_execution_codec.rs: codec failures must not collapse into display strings"
end

graph_worker_client_path = File.join(
  ROOT,
  "crates/logic_analyzer_graph_orchestration/src/worker_client.rs"
)
graph_worker_client = File.read(graph_worker_client_path).split(
  /^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/,
  2
).first
%w[new start publish].each do |operation|
  next if graph_worker_client.match?(
    /pub fn\s+#{operation}\b[^\{]*->\s*Result<[^\{]*,\s*GraphWorkerClientError>\s*\{/m
  )

  errors << "crates/logic_analyzer_graph_orchestration/src/worker_client.rs: #{operation} must retain GraphWorkerClientError"
end
unless graph_worker_client.match?(
  /pub fn\s+fail_all\b.*?error:\s*GraphWorkerTransportFailure/m
)
  errors << "crates/logic_analyzer_graph_orchestration/src/worker_client.rs: disconnects must retain GraphWorkerTransportFailure"
end

capture_export_contract = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_capture_export/src/service_contract.rs"
))
unless capture_export_contract.match?(
  /fn\s+take_completion\b.*?Result<CaptureExportCompletion,\s*CaptureExportServiceError>/m
)
  errors << "crates/logic_analyzer_capture_export/src/service_contract.rs: capture-export completion must retain its typed service error"
end
capture_export_error_path = File.join(
  ROOT,
  "crates/logic_analyzer_capture_export/src/capture_export/errors.rs"
)
capture_export_error = File.read(capture_export_error_path)
if capture_export_error.match?(/Failed\(String\)/)
  errors << "crates/logic_analyzer_capture_export/src/capture_export/errors.rs: exporter failures must not collapse into a display string"
end
%w[MissingTimelineMetadata EmptyCapture DestinationExists InvalidDestination Cancelled
   InconsistentCapture Store DestinationIo Archive].each do |variant|
  next if capture_export_error.match?(/^\s*#{variant}(?:\b|\()/)

  errors << "crates/logic_analyzer_capture_export/src/capture_export/errors.rs: CaptureExportError must classify #{variant} failures"
end
{
  "capture access" => /Capture\(\#\[source\]\s*CaptureStoreError\)/,
  "executor startup" => /Executor\(\#\[source\]\s*std::io::Error\)/,
  "worker loss" => /WorkerStopped,/,
  "export" => /Export\(CaptureExportError\)/
}.each do |boundary, pattern|
  next if capture_export_contract.match?(pattern)

  errors << "crates/logic_analyzer_capture_export/src/service_contract.rs: capture-export service must retain its #{boundary} failure"
end

ui_graph_run_contract = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/graph_service/contract.rs"
))
unless ui_graph_run_contract.match?(/fn\s+take_failure\b.*?Option<GraphRunFailure>/m)
  errors << "crates/logic_analyzer_ui/src/graph_service/contract.rs: graph-run terminal diagnostics must retain GraphRunFailure"
end
ui_app_source = File.read(File.join(ROOT, "crates/logic_analyzer_ui/src/app.rs"))
if ui_app_source.match?(/error\s*==\s*"capture export was cancelled"/)
  errors << "crates/logic_analyzer_ui/src/app.rs: capture-export cancellation must be matched by typed variant, not display text"
end

ui_output_presentation = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/collected_output_presentation.rs"
)).split(/^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/, 2).first
%w[bind_collected_output_presentations waveform_presentation_registry].each do |operation|
  next if ui_output_presentation.match?(
    /fn\s+#{operation}\b.*?Result<.*?PresentationBindingError>/m
  )

  errors << "crates/logic_analyzer_ui/src/collected_output_presentation.rs: #{operation} must retain PresentationBindingError"
end
ui_table_presentation = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/decoder_table_presentation.rs"
)).split(/^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/, 2).first
unless ui_table_presentation.match?(
  /fn\s+decoder_table_registry\b.*?Result<.*?PresentationBindingError>/m
)
  errors << "crates/logic_analyzer_ui/src/decoder_table_presentation.rs: decoder-table binding must retain PresentationBindingError"
end
if ui_output_presentation.match?(/Result<.*?,\s*String>/m) ||
   ui_table_presentation.match?(/Result<.*?,\s*String>/m)
  errors << "crates/logic_analyzer_ui/src: catalog-presentation binding must not collapse contract failures into strings"
end

def relative(path)
  path.delete_prefix("#{ROOT}/")
end

def line_number(source, offset)
  source[0...offset].count("\n") + 1
end

def implementation_source(source)
  test_module = source.index(/^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/)
  test_module.nil? ? source : source[0...test_module]
end

def test_source(path, source)
  return source if File.basename(path).include?("tests")

  test_module = source.index(/^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/)
  test_module.nil? ? "" : source[test_module..]
end

def crate_root(path)
  directory = File.dirname(path)
  loop do
    return directory if File.file?(File.join(directory, "Cargo.toml"))

    parent = File.dirname(directory)
    return nil if parent == directory

    directory = parent
  end
end

def path_dependencies_by_kind(manifest)
  dependencies = { production: [], development: [] }
  kind = nil

  manifest.each_line do |line|
    if (header = line.match(/^\s*\[([^\]]+)\]\s*$/))
      section = header[1]
      kind = if section == "dependencies" || section.match?(/\Atarget\..*\.dependencies\z/)
               :production
             elsif section == "dev-dependencies" || section.match?(/\Atarget\..*\.dev-dependencies\z/)
               :development
             end
      next
    end

    next unless kind

    dependency = line.match(/^([A-Za-z0-9_-]+)\s*=\s*\{[^}]*\bpath\s*=\s*"[^"]+"[^}]*\}/)
    dependencies[kind] << dependency[1] if dependency
  end

  dependencies.transform_values(&:uniq)
end

files = SOURCE_GLOBS.flat_map { |glob| Dir.glob(File.join(ROOT, glob)) }.sort

REQUIRED_PRIVATE_OWNER_MODULES.each do |relative_root, modules|
  source = File.read(File.join(ROOT, relative_root))
  modules.each do |name|
    next if source.match?(/^mod #{Regexp.escape(name)};$/)

    errors << "#{relative_root}: missing private owner module #{name.inspect}"
  end
end

manifest_paths = [File.join(ROOT, "Cargo.toml")] +
                 Dir.glob(File.join(ROOT, "{crates,plugins}/**/Cargo.toml")).sort
manifest_paths.each do |manifest_path|
  File.read(manifest_path).each_line.with_index(1) do |line, line_number|
    next unless line.match?(/^signal-[A-Za-z0-9_-]+\s*=\s*\{[^}]*\bworkspace\s*=\s*true/)

    errors << "#{relative(manifest_path)}:#{line_number}: internal signal crates use explicit path dependencies"
  end
end

workspace_dependencies = File.read(File.join(ROOT, "Cargo.toml"))
                             .split(/^\[package\]\s*$/, 2)
                             .first
workspace_dependencies.each_line.with_index(1) do |line, line_number|
  next unless line.match?(/^signal-[A-Za-z0-9_-]+\s*=/)

  errors << "Cargo.toml:#{line_number}: internal signal crates do not belong in workspace dependencies"
end

ui_compiler_free_functions = %w[
  apply_live_capture_edit
  derived_cache_configs_by_node
  discover_capture_presentation
  discover_live_capture_feature
  discover_trigger_configuration
  lower
  sampling_overlay_candidates
  start_app_run
  start_app_run_with_source_overrides
  start_live_analysis
  synchronize_payload_subscriptions
].freeze

graph_api_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_graph_capabilities/Cargo.toml"))
%w[
  logic-analyzer-capture-export logic-analyzer-capture-formats logic-analyzer-device-dslogic
  logic-analyzer-graph logic-analyzer-protocol-decoders logic-analyzer-ui signal-generators
  signal-sinks signal-transforms
].each do |dependency|
  if graph_api_manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
    errors << "crates/logic_analyzer_graph_capabilities/Cargo.toml: graph API must not depend on #{dependency}"
  end
end

compiler_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_graph_compiler/Cargo.toml"))
compiler_production_manifest = compiler_manifest.split(/^\[dev-dependencies\]\s*$/, 2).first
%w[
  logic-analyzer-capture-export logic-analyzer-capture-formats logic-analyzer-device-dslogic
  logic-analyzer-graph-nodes logic-analyzer-protocol-decoders logic-analyzer-test-support
  logic-analyzer-ui signal-generators signal-sinks signal-transforms tempfile thiserror zip
].each do |dependency|
  if compiler_production_manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
    errors << "crates/logic_analyzer_graph_compiler/Cargo.toml: compiler production code must not depend on #{dependency}"
  end
end

compiler_root = File.read(File.join(ROOT, "crates/logic_analyzer_graph_compiler/src/lib.rs"))
(ui_compiler_free_functions + ["BuilderRegistry"]).each do |name|
  if compiler_root.match?(/\b#{Regexp.escape(name)}\b/)
    errors << "crates/logic_analyzer_graph_compiler/src/lib.rs: compiler facade must not expose transitional #{name}"
  end
end

%w[
  logic_analyzer_capture_formats logic_analyzer_device_dslogic logic_analyzer_protocol_decoders
  signal_generators signal_sinks signal_transforms
].each do |crate|
  manifest_path = File.join(ROOT, "crates/#{crate}/Cargo.toml")
  production_manifest = File.read(manifest_path).split(/^\[dev-dependencies\]\s*$/, 2).first
  if production_manifest.match?(/^logic-analyzer-test-support\s*=/)
    errors << "crates/#{crate}/Cargo.toml: production code must not depend on test support"
  end
end

test_support_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_test_support/Cargo.toml"))
%w[
  logic-analyzer-capture-export logic-analyzer-capture-formats logic-analyzer-device-dslogic
  logic-analyzer-graph-capabilities logic-analyzer-graph-compiler logic-analyzer-graph-nodes
  logic-analyzer-protocol-decoders logic-analyzer-ui signal-generators signal-sinks signal-transforms
].each do |dependency|
  if test_support_manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
    errors << "crates/logic_analyzer_test_support/Cargo.toml: shared test support must not depend on #{dependency}"
  end
end

capture_export_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_capture_export/Cargo.toml"))
%w[
  logic-analyzer-capture-formats logic-analyzer-device-dslogic logic-analyzer-graph-capabilities
  logic-analyzer-graph-compiler logic-analyzer-graph-nodes logic-analyzer-protocol-decoders
  logic-analyzer-ui signal-generators signal-sinks signal-transforms
].each do |dependency|
  if capture_export_manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
    errors << "crates/logic_analyzer_capture_export/Cargo.toml: capture export must not depend on #{dependency}"
  end
end

graph_nodes_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_graph_nodes/Cargo.toml"))
if graph_nodes_manifest.match?(/^logic-analyzer-graph-compiler\s*=/)
  errors << "crates/logic_analyzer_graph_nodes/Cargo.toml: built-in nodes and their isolated tests must use graph API contracts, not the compiler"
end

ui_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_ui/Cargo.toml"))
if ui_manifest.match?(/^rfd\s*=/)
  errors << "crates/logic_analyzer_ui/Cargo.toml: native dialogs belong to platform"
end
%w[
  logic-analyzer-capture-formats logic-analyzer-device-dslogic
  logic-analyzer-protocol-decoders signal-generators signal-sinks signal-transforms
].each do |dependency|
  if ui_manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
    errors << "crates/logic_analyzer_ui/Cargo.toml: UI composition must not depend on concrete processing owner #{dependency}"
  end
end
node_graph_manifest = File.read(File.join(ROOT, "crates/widgets/node_graph/Cargo.toml"))
if node_graph_manifest.match?(/^rfd\s*=/) || node_graph_manifest.match?(/^native-file-dialog\s*=/)
  errors << "crates/widgets/node_graph/Cargo.toml: file dialogs must be injected through the widget-owned portable contract"
end
platform_file_dialog = File.read(File.join(ROOT, "crates/platform/src/file_dialog.rs"))
unless platform_file_dialog.match?(/pub enum FilePickerError\s*\{/)
  errors << "crates/platform/src/file_dialog.rs: the reusable picker facade must own a typed failure contract"
end
%w[take_picked import_dropped].each do |method|
  next if platform_file_dialog.match?(
    /fn\s+#{method}\b.*?Result<FileReference,\s*FilePickerError>/m
  )

  errors << "crates/platform/src/file_dialog.rs: #{method} must retain FilePickerError"
end
node_graph_file_dialog = File.read(File.join(
  ROOT,
  "crates/widgets/node_graph/src/api/control.rs"
))
unless node_graph_file_dialog.match?(/pub enum FileDialogError\s*\{.*?Host\s*\{/m)
  errors << "crates/widgets/node_graph/src/api/control.rs: the portable widget dialog facade must retain typed host failures"
end
%w[take_picked import_dropped].each do |method|
  next if node_graph_file_dialog.match?(
    /fn\s+#{method}\b.*?Result<String,\s*FileDialogError>/m
  )

  errors << "crates/widgets/node_graph/src/api/control.rs: #{method} must retain FileDialogError"
end
web_node_file_dialog = File.read(File.join(ROOT, "crates/app_web/src/node_file_dialog.rs"))
unless web_node_file_dialog.scan(/\.map_err\(FileDialogError::host\)/).length >= 2
  errors << "crates/app_web/src/node_file_dialog.rs: browser composition must preserve platform picker failures through the widget facade"
end
web_capture_worker = File.read(File.join(
  ROOT,
  "crates/app_web/src/web_capture_worker.rs"
)).split(/^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/, 2).first
unless web_capture_worker.match?(
  /fn\s+install_capture_worker\b.*?Result<BrowserWorkerClients,\s*BrowserCaptureWorkerInstallError>/m
)
  errors << "crates/app_web/src/web_capture_worker.rs: browser capture-worker installation must retain its typed lifecycle error"
end
unless web_capture_worker.match?(
  /type AttachmentComplete\s*=.*?Result<AttachedCapture,\s*BrowserCaptureAttachmentError>/m
) && web_capture_worker.match?(
  /fn\s+attach_capture_file\b.*?Result<bool,\s*BrowserCaptureAttachmentError>/m
)
  errors << "crates/app_web/src/web_capture_worker.rs: browser capture attachment submission and completion must retain typed failures"
end
web_capture_worker_errors = File.read(File.join(
  ROOT,
  "crates/app_web/src/web_capture_worker_errors.rs"
))
unless web_capture_worker_errors.match?(/CaptureClient\(\#\[source\]\s*CaptureWorkerClientError\)/) &&
       web_capture_worker_errors.match?(/GraphClient\(\#\[source\]\s*GraphWorkerClientError\)/) &&
       web_capture_worker_errors.match?(/Metadata\(\#\[source\]\s*serde_json::Error\)/)
  errors << "crates/app_web/src/web_capture_worker_errors.rs: browser worker lifecycle must retain client and metadata causes"
end
unless web_capture_worker_errors.match?(/pub\(crate\) enum BrowserWorkerTransportError\s*\{/) &&
       web_capture_worker_errors.match?(/Capture\(\#\[source\]\s*CaptureWorkerTransportFailure\)/) &&
       web_capture_worker_errors.match?(/Graph\(\#\[source\]\s*GraphWorkerTransportFailure\)/) &&
       web_capture_worker_errors.match?(/OutputFiles\(\#\[source\]\s*serde_json::Error\)/)
  errors << "crates/app_web/src/web_capture_worker_errors.rs: browser worker transport must retain neutral transport and output-codec causes"
end
%w[post_capture_request post_graph_request].each do |operation|
  unless web_capture_worker.match?(
    /fn\s+#{operation}\b.*?Result<.*?BrowserWorkerTransportError>/m
  )
    errors << "crates/app_web/src/web_capture_worker.rs: #{operation} must retain BrowserWorkerTransportError"
  end
end
web_file_registry = File.read(File.join(
  ROOT,
  "crates/app_web/src/web_file_import/registry.rs"
))
%w[
  register register_chunks register_chunks_with_identity allocate_reference
  register_worker_backed resolve
].each do |operation|
  next if web_file_registry.match?(
    /fn\s+#{operation}\b.*?Result<.*?BrowserFileRegistryError>/m
  )

  errors << "crates/app_web/src/web_file_import/registry.rs: #{operation} must retain BrowserFileRegistryError"
end
if web_file_registry.match?(/Result<.*?,\s*String>/m)
  errors << "crates/app_web/src/web_file_import/registry.rs: imported-file registry failures must not collapse into strings"
end
%w[dsl sigrok].each do |adapter|
  source = File.read(File.join(ROOT, "crates/app_web/src/web_file_import/#{adapter}.rs"))
  unless source.scan(/CaptureSourceMetadataError::access\)/).length >= 2 &&
         source.include?("CaptureSourceConstructionError::construction")
    errors << "crates/app_web/src/web_file_import/#{adapter}.rs: source adapters must retain browser registry causes"
  end
end
web_worker_source = File.read(File.join(
  ROOT,
  "crates/app_web/src/web_file_import/worker_source.rs"
))
%w[capture_metadata worker_capture decode_source byte_source].each do |operation|
  next if web_worker_source.match?(
    /fn\s+#{operation}\b.*?Result<.*?BrowserWorkerSourceError>/m
  )

  errors << "crates/app_web/src/web_file_import/worker_source.rs: #{operation} must retain BrowserWorkerSourceError"
end
if web_worker_source.match?(/Result<.*?,\s*String>/m) ||
   web_worker_source.include?("CaptureSourceMetadataError::access_message") ||
   web_worker_source.include?("CaptureSourceConstructionError::diagnostic")
  errors << "crates/app_web/src/web_file_import/worker_source.rs: worker-source failures must retain typed causes"
end
native_sigrok_catalog = File.read(File.join(
  ROOT,
  "crates/app_native/src/sigrok_catalog.rs"
))
unless native_sigrok_catalog.match?(/enum SigrokCatalogSettingsError\s*\{/) &&
       native_sigrok_catalog.match?(/fn load_settings\b.*?Result<.*?SigrokCatalogSettingsError>/m) &&
       native_sigrok_catalog.match?(/fn save_settings\b.*?Result<.*?SigrokCatalogSettingsError>/m) &&
       native_sigrok_catalog.include?("settings_error: Option<SigrokCatalogSettingsError>")
  errors << "crates/app_native/src/sigrok_catalog.rs: Sigrok settings persistence must retain typed load/save causes independently of scan diagnostics"
end
if native_sigrok_catalog.match?(/Result<.*?,\s*String>/m)
  errors << "crates/app_native/src/sigrok_catalog.rs: Sigrok settings persistence failures must not collapse into strings"
end
platform_document_error = File.read(File.join(ROOT, "crates/platform/src/document.rs"))
unless platform_document_error.match?(/pub enum DocumentError\s*\{/) &&
       platform_document_error.scan(/#\[source\]\s*\n\s*source:\s*Box<dyn Error \+ Send \+ Sync>/).length >= 3
  errors << "crates/platform/src/document.rs: document access must retain typed host causes"
end
%w[native_document web_document].each do |adapter|
  source = File.read(File.join(ROOT, "crates/platform/src/host/#{adapter}.rs"))
  %w[read write].each do |operation|
    next if source.match?(/pub fn #{operation}(?:_document)?\b.*?Result<.*?DocumentError>/m)

    errors << "crates/platform/src/host/#{adapter}.rs: #{operation} must return DocumentError"
  end
end
ui_host_contract = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/host_service/contract.rs"
))
unless ui_host_contract.match?(/pub enum GraphDocumentError\s*\{/) &&
       ui_host_contract.match?(/fn load_graph\b.*?Result<node_graph::GraphState,\s*GraphDocumentError>/m) &&
       ui_host_contract.match?(/fn save_graph\b.*?Result<\(\),\s*GraphDocumentError>/m)
  errors << "crates/logic_analyzer_ui/src/host_service/contract.rs: graph persistence must retain GraphDocumentError"
end
ui_plugin_panel_contract = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/plugin_panel/contract.rs"
))
ui_plugin_panel_error = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/plugin_panel/error.rs"
))
ui_plugin_panel_registration = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/plugin_panel/registration.rs"
))
unless ui_plugin_panel_contract.match?(
  /fn restore_state\b.*?Result<\(\),\s*PluginPanelStateError>/m
) && ui_plugin_panel_error.match?(/pub struct PluginPanelStateError\s*\{/) &&
       ui_plugin_panel_error.match?(
         /source:\s*Box<dyn StdError \+ Send \+ Sync>/
       )
  errors << "crates/logic_analyzer_ui/src/plugin_panel: panel-state restoration must retain typed plugin causes"
end
unless ui_plugin_panel_error.match?(/pub enum PluginPanelRegistrationError\s*\{/) &&
       ui_plugin_panel_registration.match?(
         /pub fn validate\b.*?Result<\(\),\s*PluginPanelRegistrationError>/m
       )
  errors << "crates/logic_analyzer_ui/src/plugin_panel: panel registration must expose its classified UI-owned error"
end
ui_capture_error = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/live_capture/error.rs"
))
ui_capture_acquisition = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/live_capture/acquisition_state.rs"
))
ui_capture_publication = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_ui/src/live_capture/storage_publication.rs"
))
unless ui_capture_error.match?(/pub\(crate\) enum CaptureCoordinatorError\s*\{/) &&
       %w[Repository Store GraphSource Waveform Executor Acquisition].all? do |variant|
         ui_capture_error.match?(/^\s*#{variant}\b/)
       end
  errors << "crates/logic_analyzer_ui/src/live_capture/error.rs: live-capture coordination must classify and retain owner-typed causes"
end
unless ui_capture_acquisition.match?(
  /fn run_capture_worker\b.*?Result<PublishedCapture,\s*CaptureCoordinatorError>/m
) && ui_capture_publication.scan(/Result<.*?CaptureCoordinatorError>/m).length >= 6
  errors << "crates/logic_analyzer_ui/src/live_capture: worker and publication paths must retain CaptureCoordinatorError"
end
%w[app_native/src/native_host.rs app_web/src/host_service.rs].each do |adapter|
  source = File.read(File.join(ROOT, "crates/#{adapter}"))
  unless source.include?("GraphDocumentError::read") && source.include?("GraphDocumentError::write")
    errors << "crates/#{adapter}: application composition must preserve platform document causes through the UI facade"
  end
end
platform_download_error = File.read(File.join(ROOT, "crates/platform/src/download.rs"))
unless platform_download_error.match?(/pub enum DownloadError\s*\{/) &&
       platform_download_error.match?(/pub enum DownloadOperation\s*\{/)
  errors << "crates/platform/src/download.rs: output downloads must classify availability and host-operation failures"
end
browser_document = File.read(File.join(ROOT, "crates/platform/src/host/web_document.rs"))
unless browser_document.match?(/pub fn download\b.*?Result<\(\),\s*DownloadError>/m) &&
       browser_document.match?(/fn download_file\b.*?Result<\(\),\s*DownloadError>/m)
  errors << "crates/platform/src/host/web_document.rs: browser output downloads must retain DownloadError"
end
unless ui_host_contract.match?(/pub enum OutputDownloadError\s*\{.*?Host\s*\{/m) &&
       ui_host_contract.match?(/fn download_output\b.*?Result<\(\),\s*OutputDownloadError>/m)
  errors << "crates/logic_analyzer_ui/src/host_service/contract.rs: output downloads must retain typed host failures"
end
web_host_service = File.read(File.join(ROOT, "crates/app_web/src/host_service.rs"))
unless web_host_service.include?("OutputDownloadError::host(id, error)")
  errors << "crates/app_web/src/host_service.rs: web composition must preserve the platform download cause"
end
platform_worker_adapter_error = File.read(File.join(ROOT, "crates/platform/src/worker_adapter.rs"))
unless platform_worker_adapter_error.match?(/pub enum WorkerAdapterError\s*\{/) &&
       platform_worker_adapter_error.include?("source: WorkerQueueError") &&
       platform_worker_adapter_error.include?("source: std::io::Error") &&
       platform_worker_adapter_error.match?(/pub enum WorkerAdapterOperation\s*\{/)
  errors << "crates/platform/src/worker_adapter.rs: worker construction must retain queue, native-start, and host-stage failures"
end
native_worker = File.read(File.join(ROOT, "crates/platform/src/host/native_worker.rs"))
unless native_worker.match?(/pub\(crate\) fn new\b.*?Result<Self,\s*WorkerAdapterError>/m) &&
       native_worker.include?("WorkerAdapterError::NativeWorkerStart")
  errors << "crates/platform/src/host/native_worker.rs: native worker construction must retain WorkerAdapterError"
end
browser_worker = File.read(File.join(ROOT, "crates/platform/src/host/web_worker.rs"))
unless browser_worker.match?(/pub fn new\b.*?Result<Self,\s*WorkerAdapterError>/m) &&
       browser_worker.include?("WorkerAdapterOperation::CreateBootstrapPayload") &&
       browser_worker.include?("WorkerAdapterOperation::StartWorker") &&
       browser_worker.match?(/enum WebWorkerMessageError\s*\{/) &&
       browser_worker.match?(/enum WebWorkerHostError\s*\{/) &&
       browser_worker.match?(/fn post_run\b.*?Result<\(\),\s*WebWorkerHostError>/m)
  errors << "crates/platform/src/host/web_worker.rs: browser worker construction must retain WorkerAdapterError"
end
if browser_worker.match?(/Result<.*?,\s*String>/m)
  errors << "crates/platform/src/host/web_worker.rs: browser worker message and submission failures must not collapse into strings before WorkerFailure serialization"
end
native_composition = File.read(File.join(ROOT, "crates/app_native/src/native.rs"))
unless native_composition.include?(
  "platform::native_worker_operation_executor(signal_derived::portable_worker_kernels())?"
)
  errors << "crates/app_native/src/native.rs: native composition must propagate worker-adapter construction failure"
end
platform_artifact_open_error = File.read(File.join(
  ROOT,
  "crates/platform/src/artifact_repository.rs"
))
unless platform_artifact_open_error.match?(/pub enum ArtifactRepositoryOpenError\s*\{/) &&
       platform_artifact_open_error.match?(/pub enum ArtifactRepositoryOpenOperation\s*\{/) &&
       platform_artifact_open_error.include?("source: RepositoryError") &&
       platform_artifact_open_error.match?(/Protocol\s*\{.*?source:\s*Box<dyn std::error::Error \+ Send \+ Sync>/m)
  errors << "crates/platform/src/artifact_repository.rs: repository opening must classify host, availability, protocol, and hydration failures"
end
browser_repository_facade = File.read(File.join(ROOT, "crates/platform/src/host/web.rs"))
unless browser_repository_facade.match?(
  /open_browser_artifact_repository\b.*?Result<Arc<dyn ArtifactRepository>,\s*ArtifactRepositoryOpenError>/m
)
  errors << "crates/platform/src/host/web.rs: browser repository facade must retain ArtifactRepositoryOpenError"
end
browser_repository = File.read(File.join(
  ROOT,
  "crates/platform/src/host/web_artifact_repository.rs"
))
%w[open initialize_worker parse_initial_state install_runtime].each do |operation|
  next if browser_repository.match?(
    /(?:fn|async fn) #{operation}\b.*?Result<.*?ArtifactRepositoryOpenError>/m
  )

  errors << "crates/platform/src/host/web_artifact_repository.rs: #{operation} must retain ArtifactRepositoryOpenError"
end
unless browser_repository.match?(/enum BrowserArtifactProtocolError\s*\{/) &&
       browser_repository.match?(/enum BrowserPersistenceCommandError\s*\{/) &&
       browser_repository.match?(/fn post_command\b.*?Result<\(\),\s*BrowserPersistenceCommandError>/m) &&
       browser_repository.match?(/fn decode_identity\b.*?Result<\[u8; 32\],\s*BrowserArtifactProtocolError>/m)
  errors << "crates/platform/src/host/web_artifact_repository.rs: browser persistence messages, commands, and identities must retain typed failures"
end
if browser_repository.match?(/Result<.*?,\s*String>/m)
  errors << "crates/platform/src/host/web_artifact_repository.rs: browser persistence failures must not collapse into strings below repository or logging boundaries"
end
native_usb = File.read(File.join(ROOT, "crates/platform/src/host/native_usb.rs"))
unless native_usb.match?(/pub enum UsbDeviceOpenError\s*\{/) &&
       native_usb.match?(/pub enum UsbDeviceOpenOperation\s*\{/) &&
       native_usb.include?("source: rusb::Error") &&
       native_usb.match?(
         /pub fn open\b.*?Result<Self,\s*UsbDeviceOpenError>/m
       )
  errors << "crates/platform/src/host/native_usb.rs: USB opening must retain selector and classified libusb failures"
end
logic_analyzer_driver = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_acquisition/src/driver.rs"
))
unless logic_analyzer_driver.match?(
  /Transport\(\#\[source\]\s*Box<dyn StdError \+ Send \+ Sync>\)/
) && logic_analyzer_driver.include?("pub fn transport(")
  errors << "crates/logic_analyzer_acquisition/src/driver.rs: driver transport failures must retain typed sources"
end
acquisition_contract = File.read(File.join(
  ROOT,
  "crates/signal_capture_session/src/live_capture/acquisition.rs"
))
unless acquisition_contract.match?(
  /Transport\(\#\[source\]\s*Box<dyn StdError \+ Send \+ Sync>\)/
) && acquisition_contract.include?("pub fn transport(")
  errors << "crates/signal_capture_session/src/live_capture/acquisition.rs: acquisition transport failures must retain typed sources"
end
native_u3pro16 = File.read(File.join(ROOT, "crates/app_native/src/u3pro16_host.rs"))
unless native_u3pro16.include?(".map_err(LogicAnalyzerError::transport)")
  errors << "crates/app_native/src/u3pro16_host.rs: native device composition must preserve the platform USB-open cause"
end
dslogic_common = File.read(File.join(
  ROOT,
  "crates/logic_analyzer_device_dslogic/src/device/dslogic_u3pro16/common.rs"
))
unless dslogic_common.include?(
  "LogicAnalyzerError::Transport(source) => AcquisitionError::Transport(source)"
)
  errors << "crates/logic_analyzer_device_dslogic: device construction must preserve typed transport causes"
end

native_app_manifest = File.read(File.join(ROOT, "crates/app_native/Cargo.toml"))
if native_app_manifest.match?(/^logic-analyzer-ui\s*=\s*\{[^}]*features\s*=/)
  errors << "crates/app_native/Cargo.toml: native UI behavior must be supplied by platform, not UI features"
end

Dir.glob(File.join(ROOT, "plugins/*/Cargo.toml")).sort.each do |manifest_path|
  manifest = File.read(manifest_path)
  production_manifest = manifest.split(/^\[dev-dependencies\]\s*$/, 2).first
  if production_manifest.match?(/^logic-analyzer-graph-compiler\s*=/)
    errors << "#{relative(manifest_path)}: plugins depend on the graph API, not the compiler"
  end
  %w[logic-analyzer-graph-compiler logic-analyzer-graph-nodes].each do |dependency|
    if manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
      errors << "#{relative(manifest_path)}: #{dependency} composition belongs in the top-level integration package"
    end
  end
end

Dir.glob(File.join(ROOT, "{crates,plugins}/**/Cargo.toml")).sort.each do |manifest_path|
  dependencies = path_dependencies_by_kind(File.read(manifest_path))
  dependencies[:development].each do |dependency|
    next if dependencies[:production].include?(dependency)
    next if dependency == "logic-analyzer-test-support"

    errors << "#{relative(manifest_path)}: test-only workspace dependency #{dependency} belongs in the top-level integration package"
  end
end

def production_path_dependencies(manifest_path)
  manifest = File.read(manifest_path).split(/^\[dev-dependencies\]\s*$/, 2).first
  manifest.scan(/^([A-Za-z0-9_-]+)\s*=\s*\{[^}]*\bpath\s*=\s*"([^"]+)"[^}]*\}/)
end

def production_rust_source(crate_directory)
  Dir.glob(File.join(crate_directory, "src/**/*.rs"))
    .sort
    .map { |path| implementation_source(File.read(path)) }
    .join("\n")
end

%w[crates/app_native crates/app_web].each do |application|
  manifest_path = File.join(ROOT, application, "Cargo.toml")
  application_source = production_rust_source(File.join(ROOT, application))
  production_path_dependencies(manifest_path).each do |dependency, relative_path|
    dependency_directory = File.expand_path(relative_path, File.dirname(manifest_path))
    next unless File.directory?(dependency_directory)

    dependency_source = production_rust_source(dependency_directory)
    next unless dependency_source.include?("inventory::submit!")

    rust_name = dependency.tr("-", "_")
    # A crate used through another production symbol is already retained by
    # the linker. Inventory-only dependencies need an explicit anchor.
    next if application_source.match?(/\b#{Regexp.escape(rust_name)}::(?!link\b)/)

    unless dependency_source.match?(/\bpub\s+fn\s+link\s*\(\s*\)\s*->\s*usize\b/)
      errors << "#{relative(manifest_path)}: inventory submitter #{dependency} must expose pub fn link() -> usize"
    end
    unless application_source.match?(/\b#{Regexp.escape(rust_name)}::link\s*\(\s*\)/)
      errors << "#{application}: enabled inventory submitter #{dependency} has no explicit linker anchor"
    end
  end
end

files.each do |path|
  rel = relative(path)
  source = File.read(path)
  tests = test_source(path, source)
  test_offset = tests.empty? ? 0 : source.index(tests)

  if rel.start_with?("crates/platform_runtime/src/")
    implementation = implementation_source(source)
    implementation.to_enum(:scan, /Result\s*<[^;{}]*,\s*String\s*>/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: platform_runtime cross-crate contracts use owner-specific error types"
    end
    implementation.to_enum(:scan, /Failed\s*\{[^}]{0,300}\bmessage:\s*String/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: worker terminal failures use the classified WorkerFailure contract"
    end
  end

  if rel.start_with?("crates/signal_runtime/src/")
    implementation = implementation_source(source)
    implementation.to_enum(:scan, /Result\s*<[^;{}]*,\s*String\s*>/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: signal_runtime production contracts use port, connection, pipeline, or work error types"
    end
    implementation.to_enum(:scan, /struct\s+NodeFailure\s*\{[^}]{0,500}\bmessage\s*:/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: supervised node failures retain the typed WorkError contract"
    end
  end

  source.to_enum(:scan, /#\s*\[\s*ignore(?:\s*=|\s*\])/).each do
    errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: benchmarks and external validation belong in explicit non-test commands, not ignored tests"
  end

  tests.to_enum(:scan, /\b(?:std::)?env::(?:var|var_os)\s*\(/).each do
    offset = test_offset + Regexp.last_match.begin(0)
    errors << "#{rel}:#{line_number(source, offset)}: portable tests must not select fixtures or behavior through environment variables"
  end

  tests.to_enum(:scan, /\b(?:SIGROK_DECODERS_DIR|DSLOGIC_U3PRO16_FPGA_IMAGE)\b/).each do
    offset = test_offset + Regexp.last_match.begin(0)
    errors << "#{rel}:#{line_number(source, offset)}: external-resource prerequisites belong in explicit developer tools"
  end

  owner = crate_root(path)
  tests.to_enum(:scan, /include_(?:str|bytes)!\s*\(\s*"([^"]+)"\s*\)/).each do
    fixture = File.expand_path(Regexp.last_match[1], File.dirname(path))
    source_offset = test_offset + Regexp.last_match.begin(0)
    unless owner && (fixture == owner || fixture.start_with?("#{owner}/"))
      errors << "#{rel}:#{line_number(source, source_offset)}: tests must not include fixtures outside their owning crate"
      next
    end

    unless system("git", "-C", ROOT, "ls-files", "--error-unmatch", relative(fixture), out: File::NULL, err: File::NULL)
      errors << "#{rel}:#{line_number(source, source_offset)}: required test fixture #{relative(fixture)} must be tracked by Git"
    end
  end

  if rel.start_with?("crates/logic_analyzer_ui/src/")
    implementation = implementation_source(source)
    host_service_adapter = rel == "crates/logic_analyzer_ui/src/host_service/native.rs"
    graph_service_adapter = rel == "crates/logic_analyzer_ui/src/graph_service/graph_compiler.rs"
    capture_export_service_adapter =
      rel.start_with?("crates/logic_analyzer_ui/src/capture_export_service/")
    unless File.basename(path).include?("tests")
      implementation.to_enum(:scan, /\b(?:trait\s+GraphService|dyn\s+GraphService)\b/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: UI graph execution uses the concrete UiGraphService; do not reintroduce a production GraphService trait"
      end
    end
    unless File.basename(path).include?("tests") || graph_service_adapter
      implementation.to_enum(:scan, /\b(?:GraphCompiler|GraphLowerer|GraphRuntime|LiveRun)\b/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: UI orchestration depends on the UI-owned UiGraphService and GraphRun; concrete lowering and runtime knowledge belongs in the graph-service owner"
      end
    end
    implementation.to_enum(:scan, /\bBuilderRegistry\b/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: UI hosts use GraphCompiler, not BuilderRegistry"
    end
    ui_compiler_free_functions.each do |function|
      implementation.to_enum(:scan, /\bcompiler::#{Regexp.escape(function)}\s*\(/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: UI hosts call GraphCompiler##{function}"
      end
    end
    unless File.basename(path).include?("tests") || host_service_adapter
      implementation.to_enum(:scan, /\brfd::/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: native dialogs belong behind the UI-owned HostService"
      end
      implementation.to_enum(:scan, /\b(?:load_from_path|save_to_path)\s*\(/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: application graph persistence belongs behind the UI-owned HostService"
      end
      implementation.to_enum(:scan, /\bsignal_derived::clear_cache(?:_entry)?\s*\(/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: cache commands belong behind the UI-owned HostService"
      end
    end
    unless File.basename(path).include?("tests")
      implementation.to_enum(
        :scan,
        /\b(?:decoded_block_cache_stats|platform_memory_snapshot)\b|\bhost_service\s*\.\s*(?:decoded_block_cache|inspect_cache_entry)\b/
      ).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: cache diagnostics use the instance-owned decoded cache and UiGraphService rather than host or platform routes"
      end
      unless capture_export_service_adapter
        implementation.to_enum(:scan, /\blogic_analyzer_capture_export\b/).each do
          errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: capture export owner details belong behind the UI CaptureExportService adapter"
        end
      end
      implementation.to_enum(:scan, /\b(?:export_finalized_capture|CaptureExportObserver|CaptureExportProgress|CaptureExportReport|ActiveExport)\b/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: capture export worker details belong behind CaptureExportService"
      end
    end
  end

  if rel.start_with?("crates/signal_sinks/src/") && !File.basename(path).include?("tests")
    implementation = implementation_source(source)
    implementation.to_enum(:scan, /\b(?:std::fs|File::create|OpenOptions)\b/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: portable sinks write through OutputStorage rather than native file I/O"
    end
  end

  if rel.start_with?("crates/logic_analyzer_ui/src/node_catalog_service/") && !File.basename(path).include?("tests")
    implementation = implementation_source(source)
    implementation.to_enum(:scan, /\bPathBuf\b/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: node catalog services expose host-formatted labels rather than filesystem paths"
    end
  end

  if rel.start_with?("crates/widgets/trigger_editor/src/") && !File.basename(path).include?("tests")
    implementation = implementation_source(source)
    ["U3Pro16", "DSLogic", "SPI", "UART", "Binary Decoder", "demo:"].each do |token|
      implementation.to_enum(:scan, /#{Regexp.escape(token)}/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: generic trigger editor contains concrete provider or protocol token #{token.inspect}"
      end
    end
  end

  graph_node_implementation = rel.start_with?("crates/logic_analyzer_graph_nodes/src/nodes/")
  plugin_implementation = rel.start_with?("plugins/")
  if graph_node_implementation || plugin_implementation
    implementation = implementation_source(source)
    implementation.to_enum(:scan, /\bCompileCtx\b/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: graph-node implementations receive NodeBuildContext, not host CompileCtx"
    end
  end

  source.to_enum(:scan, /\bpub\s*\((?:super|in\s+[^)]*)\)/).each do
    errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: pub(super) and pub(in ...) are forbidden"
  end

  declaration = /^\s*(?<visibility>pub(?:\([^)]*\))?\s+)?mod\s+(?<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?:;|\{)/
  source.to_enum(:scan, declaration).each do
    match = Regexp.last_match
    name = match[:name]
    line = line_number(source, match.begin(0))

    unless ROOT_FILES.include?(File.basename(path))
      preceding = source[[match.begin(0) - 200, 0].max...match.begin(0)]
      test_target = rel.start_with?("tests/", "benches/")
      test_module = name.include?("tests") && (test_target || preceding.match?(/#\s*\[\s*cfg\s*\([^\]]*\btest\b/))
      unless test_module
        errors << "#{rel}:#{line}: module declarations belong only in lib.rs, main.rs, or mod.rs"
      end
    end

    next unless match[:visibility]&.strip == "pub"

    allowed = PUBLIC_MODULES.fetch(rel, [])
    unless allowed.include?(name)
      errors << "#{rel}:#{line}: public module #{name.inspect} is not in the allowlist"
    end

    module_directory = File.join(File.dirname(path), name, "mod.rs")
    unless File.file?(module_directory)
      errors << "#{rel}:#{line}: public module #{name.inspect} must be directory-backed by #{relative(module_directory)}"
    end
  end

  next unless File.basename(path) == "mod.rs"

  implementation = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+)?(?:struct|enum|union|trait|fn|const|static|type)\b|^\s*impl(?:\s|<)|^\s*macro_rules!/
  source.to_enum(:scan, implementation).each do
    errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: mod.rs files may contain declarations and re-exports only"
  end
  source.to_enum(:scan, /\bcfg_select!\s*[({]/).each do
    next if DECLARATIVE_SELECTION_FACADES.include?(rel)

    errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: declaration selection macros are allowed only in approved target-selection facades"
  end
  source.to_enum(:scan, /\binclude!\s*[({]/).each do
    errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: executable include macros are not allowed in mod.rs"
  end
  source.to_enum(:scan, /^\s*use\s+/).each do
    errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: mod.rs imports must be facade re-exports"
  end

  concrete_graph_node = rel.match?(%r{\Acrates/logic_analyzer_graph_nodes/src/nodes/(?:decoders|logic|sinks|sources)/[^/]+/mod\.rs\z})
  if concrete_graph_node
    source.to_enum(:scan, /^\s*pub(?:\(crate\))?\s+use\s+/).each do
      errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: concrete graph nodes must not re-export symbols"
    end
  end
end

# Named record structs use one field visibility. This intentionally ignores
# tuple structs: their fields are positional construction APIs and rustc's
# visibility checks already cover each position.
files.each do |path|
  rel = relative(path)
  source = File.read(path)
  source.to_enum(:scan, /\bstruct\s+([A-Za-z_][A-Za-z0-9_]*)/).each do
    match = Regexp.last_match
    name = match[1]
    opening = source.index("{", match.end(0))
    terminator = source.index(";", match.end(0))
    next if opening.nil? || (!terminator.nil? && terminator < opening)

    depth = 1
    body_length = nil
    source[(opening + 1)..].each_char.with_index do |character, index|
      case character
      when "{" then depth += 1
      when "}" then depth -= 1
      end
      if depth.zero?
        body_length = index
        break
      end
    end
    next if body_length.nil?

    body = source[(opening + 1), body_length]
    field_depth = 0
    visibilities = []
    body.each_line do |line|
      if field_depth.zero? && (field = line.match(/^\s*(?:(pub(?:\(crate\))?)\s+)?[A-Za-z_][A-Za-z0-9_]*\s*:/))
        visibilities << (field[1] || "private")
      end
      field_depth += line.count("{") - line.count("}")
    end
    kinds = visibilities.uniq
    next unless kinds.length > 1

    errors << "#{rel}:#{line_number(source, match.begin(0))}: struct #{name} mixes field visibility (#{kinds.join(", ")})"
  end
end

if errors.empty?
  puts "Rust module structure matches the responsibility and visibility design."
  exit 0
end

warn errors.join("\n")
warn "#{errors.length} Rust module-structure violation#{errors.length == 1 ? "" : "s"} found."
exit 1
