# typed: false
# frozen_string_literal: true

# Sketch of the Homebrew formula cargo-dist generates and pushes to the
# andriiklymiuk/homebrew-tools tap on each release. Mirrors the corgi formula
# pattern (on_macos/on_linux x Hardware::CPU). Version + sha256 are filled in
# per-release by cargo-dist; the values below are placeholders for review.
class Golden < Formula
  desc "Run and test Postman v2.1 collections from the terminal — CLI for Golden Retriever"
  homepage "https://github.com/Andriiklymiuk/golden-cli"
  version "2.0.8"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Andriiklymiuk/golden-cli/releases/download/v2.0.8/golden-aarch64-apple-darwin.tar.xz"
      sha256 "REPLACED_BY_DIST_AARCH64_DARWIN"

      def install
        bin.install "golden", "gr"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/Andriiklymiuk/golden-cli/releases/download/v2.0.8/golden-x86_64-apple-darwin.tar.xz"
      sha256 "REPLACED_BY_DIST_X86_64_DARWIN"

      def install
        bin.install "golden", "gr"
      end
    end
  end

  on_linux do
    if Hardware::CPU.intel? && Hardware::CPU.is_64_bit?
      url "https://github.com/Andriiklymiuk/golden-cli/releases/download/v2.0.8/golden-x86_64-unknown-linux-musl.tar.xz"
      sha256 "REPLACED_BY_DIST_X86_64_LINUX_MUSL"

      def install
        bin.install "golden", "gr"
      end
    end
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/Andriiklymiuk/golden-cli/releases/download/v2.0.8/golden-aarch64-unknown-linux-musl.tar.xz"
      sha256 "REPLACED_BY_DIST_AARCH64_LINUX_MUSL"

      def install
        bin.install "golden", "gr"
      end
    end
  end

  test do
    assert_match "golden", shell_output("#{bin}/golden --version")
    assert_match "golden", shell_output("#{bin}/gr --version")
  end
end
