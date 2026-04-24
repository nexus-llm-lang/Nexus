# frozen_string_literal: true

require "tempfile"
require "open3"

# Jekyll plugin: replace ```nexus code blocks with tree-sitter-highlighted HTML.
# Requires `tree-sitter` CLI and the nexus grammar on PATH.

module NexusHighlight
  # Build input that tree-sitter can parse: top-level type/opaque/exception defs
  # stay at the top level, but bare fn signatures get wrapped in a cap block.
  def self.prepare_input(code)
  lines = code.split("\n")
  has_real_code = lines.any? { |l| l.strip.match?(/\A(cap|handler|let|import|inject|match|if|for|while|try|conc)\b/) }
  return [code, false] if has_real_code

  # Separate top-level defs from bare fn signatures
  top_lines = []   # type/opaque/exception/comment/blank
  fn_lines = []  # fn signatures
  lines.each do |line|
    t = line.strip
    if t.match?(/\Afn\b/)
      fn_lines << line
    else
      top_lines << line
    end
  end

  return [code, false] if fn_lines.empty?

  # Build: top-level defs first, then fn sigs in a port wrapper
  parts = top_lines.reject { |l| l.strip.empty? && top_lines.last == l }
  parts << "" unless parts.empty?
  parts << "cap Sig_ do"
  parts.concat(fn_lines)
  parts << "end"

  [parts.join("\n"), true]
  end

  def self.highlight(code)
  input, wrapped = prepare_input(code)

  tmp = Tempfile.new(["nexus", ".nx"])
  tmp.write(input)
  tmp.flush

  html, status = Open3.capture2("tree-sitter", "highlight", "--html", "--css-classes", tmp.path)
  tmp.close
  tmp.unlink

  return nil unless status.success?

  lines = html.scan(/<td class=line>(.*?)<\/td>/m).map(&:first)

  if wrapped
    # Remove the "cap Sig_ do" and "end" wrapper lines, and the blank line before the cap
    # Find and remove wrapper lines by content
    lines.reject! { |l| l.match?(/<span class='keyword'>cap<\/span>.*Sig_/) || l.strip == "<span class='keyword'>end</span>" }
    # Remove trailing blank line left by the separator
    lines.pop while lines.last&.strip&.empty?
  end

  lines.join("\n")
  end
end

Jekyll::Hooks.register [:pages, :documents], :pre_render do |doc|
  doc.content = doc.content.gsub(/```nexus\s*\n(.*?)```/m) do
    code = Regexp.last_match(1).rstrip
    highlighted = NexusHighlight.highlight(code)
    if highlighted
      %(<div class="highlight-nexus"><pre><code>#{highlighted}</code></pre></div>)
    else
      Regexp.last_match(0)
    end
  end
end
