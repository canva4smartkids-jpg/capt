# Validates a media file with ffprobe and writes the ffprobe JSON next to it.
# Used by .github/workflows/windows.yml. Fails (throws) on any mismatch.
#
#   Assert-Media.ps1 -Path out.mp4 -Codec h264 -MinDuration 4.5 -MaxDuration 5.5
#   Assert-Media.ps1 -Path shot.png -Codec png -Width 1024 -Height 768
param(
  [Parameter(Mandatory = $true)][string]$Path,
  [Parameter(Mandatory = $true)][string]$Codec,
  [double]$MinDuration = -1,
  [double]$MaxDuration = -1,
  [int]$Width = 0,
  [int]$Height = 0,
  [int]$MinFrames = 0
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Path)) { throw "missing media file: $Path" }
$size = (Get-Item -LiteralPath $Path).Length
if ($size -le 0) { throw "empty media file: $Path" }

$json = & ffprobe -v error -show_streams -show_format -of json $Path
if ($LASTEXITCODE -ne 0) { throw "ffprobe failed ($LASTEXITCODE) on $Path" }
$jsonPath = "$Path.ffprobe.json"
$json | Out-File -LiteralPath $jsonPath -Encoding utf8
$probe = ($json -join "`n") | ConvertFrom-Json

$video = @($probe.streams | Where-Object { $_.codec_type -eq 'video' }) | Select-Object -First 1
if (-not $video) { throw "no video stream in $Path" }
$errors = @()
if ($video.codec_name -ne $Codec) { $errors += "codec is '$($video.codec_name)', expected '$Codec'" }
if ([int]$video.width -le 0 -or [int]$video.height -le 0) { $errors += "invalid dimensions $($video.width)x$($video.height)" }
if ($Width -gt 0 -and [int]$video.width -ne $Width) { $errors += "width $($video.width) != $Width" }
if ($Height -gt 0 -and [int]$video.height -ne $Height) { $errors += "height $($video.height) != $Height" }
$duration = $null
if ($probe.format.duration) { $duration = [double]::Parse($probe.format.duration, [Globalization.CultureInfo]::InvariantCulture) }
if ($MinDuration -ge 0 -and ($null -eq $duration -or $duration -lt $MinDuration)) { $errors += "duration $duration < $MinDuration" }
if ($MaxDuration -ge 0 -and ($null -eq $duration -or $duration -gt $MaxDuration)) { $errors += "duration $duration > $MaxDuration" }
$frames = 0
if ($video.nb_frames) { $frames = [int]$video.nb_frames }
if ($MinFrames -gt 0 -and $frames -lt $MinFrames) { $errors += "only $frames frames (< $MinFrames)" }

$summary = "{0}: {1} {2}x{3}, duration={4}s, frames={5}, size={6} bytes, format={7}" -f `
  (Split-Path -Leaf $Path), $video.codec_name, $video.width, $video.height, $duration, $frames, $size, $probe.format.format_name
Write-Host $summary
if ($env:GITHUB_STEP_SUMMARY) { "- ``$summary``" | Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8 }
if ($errors.Count -gt 0) { throw ("ffprobe validation failed for {0}: {1}" -f $Path, ($errors -join '; ')) }
