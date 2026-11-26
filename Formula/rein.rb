class Rein < Formula
  desc "Small language for agent decision processes"
  homepage "https://rein-lang.com"
  url "https://github.com/rein-lang/rein/archive/refs/tags/v0.1.6.tar.gz"
  sha256 "0ff3cda8ac4db35f20c92f89740ebd2fcc6768217cf7cfce2946609b8f8f5659"
  license "MIT"
  head "https://github.com/rein-lang/rein.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/rein")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/rein --version")
  end
end
