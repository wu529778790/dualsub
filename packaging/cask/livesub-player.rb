# LiveSub-Player 的 Homebrew cask
#
# 使用方式：把本文件复制到你的 homebrew-tap 仓库 Casks/ 目录：
#   1. 在 GitHub 建仓库 wu529778790/homebrew-tap
#   2. 复制本文件到该仓库 Casks/livesub-player.rb 并提交
#   3. 用户安装：
#        brew tap wu529778790/tap https://github.com/wu529778790/homebrew-tap
#        brew install --cask livesub-player
#
# 发新版时的更新步骤（每次 release 后执行一次）：
#   URL="https://github.com/wu529778790/livesub-player/releases/download/v<新版本>/LiveSub-Player-macOS.zip"
#   curl -fsSL -o /tmp/lsp.zip "$URL" && shasum -a 256 /tmp/lsp.zip && rm /tmp/lsp.zip
#   把新版本号、新 sha256 更新到下面的 version/sha256 字段并提交，
#   用户 `brew upgrade --cask livesub-player` 即可拿到新版。

cask "livesub-player" do
  arch arm: "arm64"

  version "0.2.1"
  sha256 "0d56cd5776529112aefc7e8adc65925053f124a656eb4d1f33061d59daf56c97"

  url "https://github.com/wu529778790/livesub-player/releases/download/v#{version}-alpha/LiveSub-Player-macOS.zip"
  name "LiveSub-Player"
  desc "实时语音识别 + 双语字幕的跨平台桌面播放器"
  homepage "https://github.com/wu529778790/livesub-player"

  # Homebrew 下载的文件不带 quarantine 标记，装完双击即开，无 Gatekeeper 拦截
  app "LiveSub-Player.app"

  livecheck do
    url "https://api.github.com/repos/wu529778790/livesub-player/releases?per_page=1"
    regex(/"tag_name":\s*"v?(\d+(?:\.\d+)+-alpha(?:\.\d+)?)"/i)
    strategy :json do |json, regex|
      json.map do |release|
        release["tag_name"]&.[](regex, 1)
      end
    end
  end

  depends_on mac: ">= :big_sur" # MACOSX_DEPLOYMENT_TARGET=11.0，仅 Apple Silicon

  caveats <<~EOS
    当前 Release 仅提供 Apple Silicon（arm64）构建，Intel Mac 暂不支持。
  EOS
end
