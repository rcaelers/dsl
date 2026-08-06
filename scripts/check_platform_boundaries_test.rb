#!/usr/bin/env ruby
# frozen_string_literal: true

require "fileutils"
require "minitest/autorun"
require "tmpdir"

require_relative "check_platform_boundaries"

class PlatformBoundaryCheckTest < Minitest::Test
  def test_rejects_target_selection_in_reusable_source
    with_workspace do |root|
      write(root, "crates/core/src/lib.rs", <<~RUST)
        #[cfg(target_arch = "wasm32")]
        mod web;
        const IS_WEB: bool = cfg!(target_arch = "wasm32");
        std::cfg_select! { target_arch = "wasm32" => {} _ => {} }
      RUST

      errors = PlatformBoundaryCheck.new(root).errors
      assert(errors.any? { |error| error.include?("select behavior") })
      assert(errors.any? { |error| error.include?("inspect the compilation target") })
      assert(errors.any? { |error| error.include?("target-selection macros") })
    end
  end

  def test_rejects_target_dependencies_in_reusable_crates
    with_workspace do |root|
      write(root, "crates/core/Cargo.toml", <<~TOML)
        [package]
        name = "core"
        [target.'cfg(target_arch = "wasm32")'.dependencies]
        web-sys = "1"
      TOML

      errors = PlatformBoundaryCheck.new(root).errors
      assert(errors.any? { |error| error.include?("target-specific dependency sections") })
      assert(errors.any? { |error| error.include?("target-specific dependency web-sys") })
    end
  end

  def test_allows_platform_bootstrap_and_documented_processing_adapter_selection
    with_workspace do |root|
      write(root, "crates/platform/src/lib.rs", <<~RUST)
        #[cfg(target_arch = "wasm32")]
        mod web;
      RUST
      write(root, "crates/platform/Cargo.toml", <<~TOML)
        [target.'cfg(target_arch = "wasm32")'.dependencies]
        web-sys = "1"
      TOML
      write(
        root,
        "crates/logic_analyzer_device_dslogic/src/device/dslogic_u3pro16/mod.rs",
        "std::cfg_select! { target_arch = \"wasm32\" => {} _ => { mod implementation; } }\n"
      )

      assert_empty(PlatformBoundaryCheck.new(root).errors)
    end
  end

  def test_rejects_core_platform_dependencies_and_allows_application_composition_dependencies
    with_workspace do |root|
      write(root, "crates/core/Cargo.toml", <<~TOML)
        [dependencies]
        platform = { path = "../platform" }
      TOML
      write(root, "crates/app_web/Cargo.toml", <<~TOML)
        [dependencies]
        logic-analyzer-device-dslogic = { path = "../logic_analyzer_device_dslogic" }
      TOML

      errors = PlatformBoundaryCheck.new(root).errors
      assert(errors.any? { |error| error.include?("core crates must not depend") })
      refute(errors.any? { |error| error.include?("logic-analyzer-device-dslogic") })
    end
  end

  def test_rejects_target_selected_test_substitutes
    with_workspace do |root|
      write(root, "crates/core/src/lib.rs", <<~RUST)
        #[cfg(target_arch = "wasm32")]
        fn source() -> SyntheticSource { SyntheticSource::new() }
      RUST

      errors = PlatformBoundaryCheck.new(root).errors
      assert(errors.any? { |error| error.include?("selected explicitly") })
    end
  end

  private

  def with_workspace
    Dir.mktmpdir("platform-boundary-test") { |root| yield(root) }
  end

  def write(root, relative, source)
    path = File.join(root, relative)
    FileUtils.mkdir_p(File.dirname(path))
    File.write(path, source)
  end
end
