class AiUsageTui < Formula
  desc "A btop-inspired dashboard for local and hosted AI usage"
  homepage "https://github.com/SophanaSok/ai-usage-tui"
  url "https://github.com/SophanaSok/ai-usage-tui/releases/download/v0.2.0/ai-usage-tui-0.2.0-x86_64-linux.tar.gz"
  sha256 "PLACEHOLDER_SHA256"
  license "MIT"

  def install
    bin.install "ai-usage-tui"
  end

  test do
    assert_match "ai-usage-tui", shell_output("#{bin}/ai-usage-tui --version")
  end
end