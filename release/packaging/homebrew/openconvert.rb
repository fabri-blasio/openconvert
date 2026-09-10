# Homebrew cask — OpenConvert
#
# Submitted to homebrew/homebrew-cask as Casks/t/openconvert.rb.
#
#   brew install --cask openconvert
#
# The two sha256 values are written by the release job from the built .dmg
# files. The zeroes are the shape of a hash, not a hash.
#
# Audit before submitting:
#   brew audit --cask --new packaging/homebrew/openconvert.rb
#   brew style packaging/homebrew/openconvert.rb

cask "openconvert" do
  version "0.1.0"

  on_arm do
    sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    url "https://github.com/fabri-blasio/cloudconvert/releases/download/v#{version}/openconvert-#{version}-aarch64-apple-darwin.dmg",
        verified: "github.com/fabri-blasio/cloudconvert/"
  end
  on_intel do
    sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    url "https://github.com/fabri-blasio/cloudconvert/releases/download/v#{version}/openconvert-#{version}-x86_64-apple-darwin.dmg",
        verified: "github.com/fabri-blasio/cloudconvert/"
  end

  name "OpenConvert"
  desc "Convert any file into anything, on your machine"
  homepage "https://openconvert.dev/"

  livecheck do
    url :url
    strategy :github_latest
  end

  # macOS 10.15 is the floor: the App Sandbox behaviour the worker relies on,
  # and notarization, both assume Catalina or later.
  depends_on macos: ">= :catalina"

  app "OpenConvert.app"
  binary "#{appdir}/OpenConvert.app/Contents/MacOS/openconvert"

  # Everything under here is written by the app itself and is the user's:
  # config, conversion history, saved recipes, the batch journal and the engine
  # quarantine list. `zap` removes them; `uninstall` deliberately does not, so
  # reinstalling does not silently discard someone's recipes.
  zap trash: [
    "~/Library/Application Support/dev.openconvert.openconvert",
    "~/Library/Caches/dev.openconvert.openconvert",
    "~/Library/Preferences/dev.openconvert.openconvert.plist",
    "~/Library/Saved Application State/dev.openconvert.openconvert.savedState",
  ]
end
