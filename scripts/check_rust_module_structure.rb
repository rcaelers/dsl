#!/usr/bin/env ruby
# frozen_string_literal: true

# Enforces docs/RESPONSIBILITY_AND_VISIBILITY_DESIGN.md. This deliberately
# checks source structure rather than formatting; rustc remains authoritative
# for name resolution and `unreachable_pub`.

ROOT = File.expand_path("..", __dir__)
SOURCE_GLOBS = ["crates/**/*.rs", "plugins/**/*.rs", "tests/**/*.rs", "benches/**/*.rs"].freeze
ROOT_FILES = %w[lib.rs main.rs mod.rs].freeze
DECLARATIVE_SELECTION_FACADES = %w[
  crates/logic_analyzer_platform/src/platform/mod.rs
  crates/logic_analyzer_processing/src/nodes/sources/dslogic_u3pro16/mod.rs
].freeze

PUBLIC_MODULES = {
  "crates/signal_derived/src/lib.rs" => %w[derived_word_store],
  "crates/signal_capture_session/src/lib.rs" => %w[live_capture live_capture_store logic_analyzer],
  "crates/logic_analyzer_processing/src/lib.rs" => %w[nodes support types],
  "crates/logic_analyzer_processing/src/support/mod.rs" => %w[logic_analyzer],
  "crates/logic_analyzer_processing/src/nodes/mod.rs" => %w[decoders logic sinks sources],
  "crates/logic_analyzer_processing/src/nodes/decoders/mod.rs" => %w[i2c_decoder parallel_decoder sigrok_decoder spi_decoder uart_decoder],
  "crates/logic_analyzer_processing/src/nodes/logic/mod.rs" => %w[
    buffer edge_detector event_control event_gate logic_gate packet_framer sr_latch text_formatter
    timeline_marker trigger_counter word_field_extractor word_matcher
  ],
  "crates/logic_analyzer_processing/src/nodes/sinks/mod.rs" => %w[
    binary_file_writer csv_word_writer discard_writer text_file_writer tgck_recorder
  ],
  "crates/logic_analyzer_processing/src/nodes/sources/mod.rs" => %w[
    dsl_file dslogic_u3pro16 sigrok_file synthetic_capture_source synthetic_uart_source
  ],
  "crates/logic_analyzer_graph_capabilities/src/lib.rs" => %w[node node_support],
  "crates/widgets/node_graph/src/lib.rs" => %w[api]
}.freeze

errors = []

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
%w[logic-analyzer-capture-export logic-analyzer-graph logic-analyzer-processing logic-analyzer-ui].each do |dependency|
  if graph_api_manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
    errors << "crates/logic_analyzer_graph_capabilities/Cargo.toml: graph API must not depend on #{dependency}"
  end
end

compiler_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_graph_compiler/Cargo.toml"))
compiler_production_manifest = compiler_manifest.split(/^\[dev-dependencies\]\s*$/, 2).first
%w[
  logic-analyzer-capture-export logic-analyzer-graph-nodes logic-analyzer-processing
  logic-analyzer-test-support logic-analyzer-ui tempfile thiserror zip
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

processing_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_processing/Cargo.toml"))
processing_production_manifest = processing_manifest.split(/^\[dev-dependencies\]\s*$/, 2).first
if processing_production_manifest.match?(/^logic-analyzer-test-support\s*=/)
  errors << "crates/logic_analyzer_processing/Cargo.toml: production processing code must not depend on test support"
end

test_support_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_test_support/Cargo.toml"))
%w[logic-analyzer-capture-export logic-analyzer-graph-capabilities logic-analyzer-graph-compiler logic-analyzer-graph-nodes logic-analyzer-processing logic-analyzer-ui].each do |dependency|
  if test_support_manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
    errors << "crates/logic_analyzer_test_support/Cargo.toml: shared test support must not depend on #{dependency}"
  end
end

capture_export_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_capture_export/Cargo.toml"))
%w[logic-analyzer-graph-capabilities logic-analyzer-graph-compiler logic-analyzer-graph-nodes logic-analyzer-processing logic-analyzer-ui].each do |dependency|
  if capture_export_manifest.match?(/^#{Regexp.escape(dependency)}\s*=/)
    errors << "crates/logic_analyzer_capture_export/Cargo.toml: capture export must not depend on #{dependency}"
  end
end

graph_nodes_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_graph_nodes/Cargo.toml"))
if graph_nodes_manifest.match?(/^logic-analyzer-graph-compiler\s*=/)
  errors << "crates/logic_analyzer_graph_nodes/Cargo.toml: built-in nodes and their isolated tests must use graph API contracts, not the compiler"
end

ui_manifest = File.read(File.join(ROOT, "crates/logic_analyzer_ui/Cargo.toml"))
if ui_manifest.match?(/^logic-analyzer-capture-export\s*=/)
  errors << "crates/logic_analyzer_ui/Cargo.toml: concrete capture export belongs to logic-analyzer-platform"
end
if ui_manifest.match?(/^rfd\s*=/)
  errors << "crates/logic_analyzer_ui/Cargo.toml: native dialogs belong to logic-analyzer-platform"
end
node_graph_manifest = File.read(File.join(ROOT, "crates/widgets/node_graph/Cargo.toml"))
if node_graph_manifest.match?(/^rfd\s*=/) || node_graph_manifest.match?(/^native-file-dialog\s*=/)
  errors << "crates/widgets/node_graph/Cargo.toml: file dialogs must be injected through the widget-owned portable contract"
end

native_app_manifest = File.read(File.join(ROOT, "crates/app_native/Cargo.toml"))
if native_app_manifest.match?(/^logic-analyzer-ui\s*=\s*\{[^}]*features\s*=/)
  errors << "crates/app_native/Cargo.toml: native UI behavior must be supplied by logic-analyzer-platform, not UI features"
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
    graph_service_adapter = %w[
      crates/logic_analyzer_ui/src/graph_service/graph_compiler.rs
      crates/logic_analyzer_ui/src/graph_service/platform_graph_compiler_native.rs
      crates/logic_analyzer_ui/src/graph_service/platform_graph_compiler_wasm.rs
    ].include?(rel)
    unless File.basename(path).include?("tests") || graph_service_adapter
      implementation.to_enum(:scan, /\b(?:GraphCompiler|LiveRun)\b/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: UI orchestration depends on the UI-owned GraphService and GraphRun; concrete compiler knowledge belongs in its adapter"
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
      implementation.to_enum(:scan, /\blogic_analyzer_capture_export\b/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: concrete capture export belongs behind CaptureExportService"
      end
      implementation.to_enum(:scan, /\b(?:export_finalized_capture|CaptureExportObserver|CaptureExportProgress|CaptureExportReport|ActiveExport)\b/).each do
        errors << "#{rel}:#{line_number(source, Regexp.last_match.begin(0))}: capture export worker details belong behind CaptureExportService"
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
