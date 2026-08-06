#!/usr/bin/env ruby
# frozen_string_literal: true

# Enforces the native/web ownership rules documented in
# docs/aspects/responsibility_visibility.md. Rust compilation proves that
# each selected target is valid; this check proves that reusable crates do not
# select different source or dependency trees in the first place.

class PlatformBoundaryCheck
  GENERAL_TARGET_ROOTS = %w[
    crates/app_native
    crates/app_web
    crates/platform
  ].freeze
  PROCESSING_ADAPTER_ALLOWLIST = %w[
    crates/logic_analyzer_processing/src/support/capture_archive/file_byte_source.rs
    crates/logic_analyzer_processing/src/nodes/sources/dsl_file/path_compatibility.rs
    crates/logic_analyzer_processing/src/nodes/sources/sigrok_file/path_compatibility.rs
    crates/logic_analyzer_processing/src/nodes/sources/dslogic_u3pro16/mod.rs
    crates/logic_analyzer_processing/src/bin/u3pro16-streaming-bench/main.rs
  ].freeze
  TARGET_DEPENDENCIES = %w[
    js-sys
    memmap2
    objc2
    objc2-app-kit
    objc2-foundation
    pyo3
    rfd
    rusb
    wasm-bindgen
    wasm-bindgen-futures
    web-sys
  ].freeze
  TARGET_PREDICATE = /\b(?:target_(?:abi|arch|endian|env|family|feature|os|pointer_width|vendor)|unix|windows)\b/
  TARGET_ATTRIBUTE = /#\s*\[\s*cfg(?:_attr)?\s*\(.*?\)\s*\]/m
  TARGET_INSPECTION = /\bcfg!\s*\(.*?\)/m
  TARGET_SELECTION_MACRO = /\b(?:std::)?cfg_select!\s*[({]/
  TARGET_MODULE = /^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(?:native|unix|wasm|web|windows)\b/
  TARGET_PATH = /#\s*\[\s*path\s*=\s*"[^"]*(?:native|unix|wasm|web|windows)[^"]*"\s*\]/

  def initialize(root)
    @root = File.expand_path(root)
  end

  def errors
    source_errors + manifest_errors
  end

  private

  def source_errors
    source_files.flat_map do |path|
      relative = relative(path)
      next [] if test_source?(relative)

      source = implementation_source(File.read(path))
      next [] if source.empty?

      errors = []
      unless target_selection_allowed?(relative)
        each_match(source, TARGET_ATTRIBUTE) do |match, line, _match_end|
          next unless match.match?(TARGET_PREDICATE)

          errors << "#{relative}:#{line}: reusable source must not select behavior by compilation target"
        end
        each_match(source, TARGET_INSPECTION) do |match, line, _match_end|
          next unless match.match?(TARGET_PREDICATE)

          errors << "#{relative}:#{line}: reusable source must not inspect the compilation target with cfg!"
        end
        each_match(source, TARGET_SELECTION_MACRO) do |_match, line, _match_end|
          errors << "#{relative}:#{line}: target-selection macros belong in an approved platform facade"
        end
        each_match(source, TARGET_MODULE) do |_match, line, _match_end|
          errors << "#{relative}:#{line}: target-specific modules belong in platform"
        end
        each_match(source, TARGET_PATH) do |_match, line, _match_end|
          errors << "#{relative}:#{line}: target-selected module paths belong in platform"
        end
      end

      each_match(source, TARGET_ATTRIBUTE) do |match, line, match_end|
        next unless match.match?(TARGET_PREDICATE)

        following = source[match_end, 600].to_s
        next unless following.match?(/\b(?:Discard\w*|Synthetic\w*)\b/)

        errors << "#{relative}:#{line}: synthetic sources and discard sinks must be selected explicitly, not by target"
      end
      errors
    end
  end

  def manifest_errors
    manifest_files.flat_map do |path|
      relative = relative(path)
      crate_root = File.dirname(relative)
      source = File.read(path)
      errors = []

      unless general_target_root?(crate_root)
        source.each_line.with_index(1) do |line, number|
          if line.match?(/^\s*\[target\..*\.(?:build-|dev-)?dependencies\]\s*$/)
            errors << "#{relative}:#{number}: reusable crates must not have target-specific dependency sections"
          end
          dependency = line.match(/^\s*([A-Za-z0-9_-]+)\s*=/)&.[](1)
          if dependency && TARGET_DEPENDENCIES.include?(dependency)
            errors << "#{relative}:#{number}: target-specific dependency #{dependency} belongs in platform"
          end
        end
      end

      if !%w[crates/app_native crates/app_web].include?(crate_root) &&
          source.match?(/^\s*platform\s*=/)
        errors << "#{relative}: reusable core crates must not depend on platform"
      end

      errors
    end
  end

  def source_files
    %w[crates/**/*.rs plugins/**/*.rs]
      .flat_map { |glob| Dir.glob(File.join(@root, glob)) }
      .sort
  end

  def manifest_files
    %w[crates/**/Cargo.toml plugins/**/Cargo.toml]
      .flat_map { |glob| Dir.glob(File.join(@root, glob)) }
      .sort
  end

  def target_selection_allowed?(relative)
    GENERAL_TARGET_ROOTS.any? { |root| relative.start_with?("#{root}/") } ||
      PROCESSING_ADAPTER_ALLOWLIST.include?(relative)
  end

  def general_target_root?(relative)
    GENERAL_TARGET_ROOTS.include?(relative)
  end

  def test_source?(relative)
    basename = File.basename(relative, ".rs")
    basename.include?("tests") || relative.split("/").include?("tests")
  end

  def implementation_source(source)
    test_module = source.index(/^\s*#\s*\[\s*cfg\s*\([^\]]*\btest\b[^\]]*\)\s*\]\s*\n\s*mod\s+\w*tests\b/)
    test_module.nil? ? source : source[0...test_module]
  end

  def each_match(source, pattern)
    source.to_enum(:scan, pattern).each do
      match = Regexp.last_match
      yield(match[0], source[0...match.begin(0)].count("\n") + 1, match.end(0))
    end
  end

  def relative(path)
    path.delete_prefix("#{@root}/")
  end
end

if $PROGRAM_NAME == __FILE__
  root = File.expand_path("..", __dir__)
  errors = PlatformBoundaryCheck.new(root).errors
  if errors.empty?
    puts "Rust platform boundaries match the unified native/web data-plane design."
    exit 0
  end

  warn errors.join("\n")
  warn "#{errors.length} Rust platform-boundary violation#{errors.length == 1 ? "" : "s"} found."
  exit 1
end
