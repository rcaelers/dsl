#!/usr/bin/env ruby
# frozen_string_literal: true

# Verifies that every public function and public-trait method has a Rustdoc comment.
# Type, field, and variant documentation have independent review requirements.

paths = `rg -l '^\s*(pub\s+)?(?:async\s+)?(?:const\s+)?fn\s+' crates plugins --glob '*.rs'`
  .lines
  .map(&:strip)

def documented?(lines, index)
  index -= 1
  while index >= 0 && lines[index].strip.start_with?('#')
    index -= 1
  end
  index >= 0 && lines[index].lstrip.start_with?('///')
end

violations = []

paths.each do |path|
  lines = File.readlines(path)
  depth = 0
  trait_depth = nil
  trait_body_pending = false

  lines.each_with_index do |line, index|
    if line.match?(/^\s*pub trait\s+/)
      trait_body_pending = true
    end
    if trait_body_pending && line.include?('{')
      trait_depth = depth + 1
      trait_body_pending = false
    end

    public_function = line.match?(/^\s*pub\s+(?:async\s+)?(?:const\s+)?fn\s+/)
    public_trait_method = trait_depth && line.match?(/^\s*(?:async\s+)?(?:const\s+)?fn\s+/)
    if (public_function || public_trait_method) && !documented?(lines, index)
      violations << "#{path}:#{index + 1}: #{line.strip}"
    end

    depth += line.count('{') - line.count('}')
    trait_depth = nil if trait_depth && depth < trait_depth
  end
end

unless violations.empty?
  warn "public functions and public trait methods require Rustdoc:\n#{violations.join("\n")}" 
  exit 1
end
