# Rendered by .github/workflows/release.yml; placeholders are substituted at release time.
# Do not hand-edit the version or checksums -- they drifted from the published artifact names
# before (the tarball is tagged v0.2.0 but this file asked for 0.2.0) and every URL 404'd.
class AiUsageTui < Formula
  desc "__DESCRIPTION__"
  homepage "https://github.com/SophanaSok/ai-usage-tui"
  version "__VERSION__"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/SophanaSok/ai-usage-tui/releases/download/__TAG__/ai-usage-tui-__TAG__-aarch64-macos.tar.gz"
      sha256 "__MACOS_ARM_SHA256__"
    end
    on_intel do
      url "https://github.com/SophanaSok/ai-usage-tui/releases/download/__TAG__/ai-usage-tui-__TAG__-x86_64-macos.tar.gz"
      sha256 "__MACOS_X86_SHA256__"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/SophanaSok/ai-usage-tui/releases/download/__TAG__/ai-usage-tui-__TAG__-x86_64-linux.tar.gz"
      sha256 "__LINUX_SHA256__"
    end
    # The aarch64-linux tarball has been built and published since v0.2.0; the formula simply
    # never offered it, so `brew install` on an ARM Linux box fell through to no bottle at all.
    on_arm do
      url "https://github.com/SophanaSok/ai-usage-tui/releases/download/__TAG__/ai-usage-tui-__TAG__-aarch64-linux.tar.gz"
      sha256 "__LINUX_ARM_SHA256__"
    end
  end

  def install
    bin.install "ai-usage-tui"
  end

  test do
    assert_match "ai-usage-tui", shell_output("#{bin}/ai-usage-tui --version")
  end
end
