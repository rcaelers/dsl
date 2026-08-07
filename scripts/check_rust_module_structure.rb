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
%w[Discovery Configuration Transport].each do |variant|
  next if sigrok_decoder_error.match?(/^\s*#{variant}\(String\),$/)

  errors << "crates/logic_analyzer_protocol_decoders/src/sigrok_decoder/runtime.rs: SigrokDecoderRuntimeError must classify #{variant.downcase} failures"
end

source_preparation_contract_path = File.join(
  ROOT,
  "crates/logic_analyzer_graph_runtime/src/runtime/source_preparation_contract.rs"
)
source_preparation_contract = File.read(source_preparation_contract_path)
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
