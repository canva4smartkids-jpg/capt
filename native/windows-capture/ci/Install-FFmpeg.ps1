# Installs ffmpeg/ffprobe on a GitHub Windows runner (Chocolatey, falling back to the gyan.dev
# release build), puts the real binaries (not Chocolatey shims) first on PATH for later steps and
# exports FFMPEG/FFPROBE/CAPTUREONE_FFMPEG/CAPTUREONE_FFPROBE.
$ErrorActionPreference = 'Stop'

function Find-RealFfmpeg {
  $candidates = @()
  if (Test-Path 'C:\ProgramData\chocolatey\lib') {
    $candidates += Get-ChildItem 'C:\ProgramData\chocolatey\lib' -Recurse -Filter ffmpeg.exe -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -notmatch '\\shims?\\' }
  }
  if ($env:RUNNER_TEMP -and (Test-Path "$env:RUNNER_TEMP\ffmpeg")) {
    $candidates += Get-ChildItem "$env:RUNNER_TEMP\ffmpeg" -Recurse -Filter ffmpeg.exe -ErrorAction SilentlyContinue
  }
  $candidates | Where-Object { Test-Path (Join-Path $_.DirectoryName 'ffprobe.exe') } | Select-Object -First 1
}

$found = Find-RealFfmpeg
if (-not $found) {
  choco install ffmpeg -y --no-progress | Out-Host
  $found = Find-RealFfmpeg
}
if (-not $found) {
  Write-Host 'Chocolatey ffmpeg unavailable; downloading the gyan.dev release build'
  $zip = Join-Path $env:RUNNER_TEMP 'ffmpeg.zip'
  Invoke-WebRequest -Uri 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip' -OutFile $zip
  Expand-Archive -Path $zip -DestinationPath (Join-Path $env:RUNNER_TEMP 'ffmpeg') -Force
  $found = Find-RealFfmpeg
}
if (-not $found) { throw 'could not install ffmpeg' }

$dir = $found.DirectoryName
$env:PATH = "$dir;$env:PATH"
if ($env:GITHUB_PATH) { $dir | Out-File -FilePath $env:GITHUB_PATH -Append -Encoding utf8 }
if ($env:GITHUB_ENV) {
  "FFMPEG=$dir\ffmpeg.exe", "FFPROBE=$dir\ffprobe.exe", "CAPTUREONE_FFMPEG=$dir\ffmpeg.exe", "CAPTUREONE_FFPROBE=$dir\ffprobe.exe" |
    Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
}
& "$dir\ffmpeg.exe" -hide_banner -version | Select-Object -First 1
Write-Host 'H.264/HEVC encoders available in this build:'
& "$dir\ffmpeg.exe" -hide_banner -encoders | Select-String -Pattern 'libx264|libx265|h264_mf|hevc_mf|nvenc|qsv|amf'
