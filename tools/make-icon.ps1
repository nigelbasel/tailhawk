# Draws Tailhawk's application icon and writes it at every size the shell asks for.
#
# The artwork is *code* rather than a binary someone once exported, so it can be re-cut when the
# palette moves or a new size is wanted, and so a reviewer can see what the mark is made of. Run
# it from the repo root; it writes `assets/`.
#
#     powershell -NoProfile -File tools/make-icon.ps1
#
# The mark is a hawk seen from above with its wings swept back — a raptor silhouette rather than a
# generic bird, and a solid one, because at 16 px a shape with thin parts is a smudge. Everything
# is proportioned in a 100x100 box and scaled, so every size is the same drawing and not a
# separately-fudged one.

[CmdletBinding()]
param(
    [string]$OutDir
)

Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'

if (-not $OutDir) {
    $OutDir = Join-Path (Split-Path -Parent (Split-Path -Parent $PSCommandPath)) 'assets'
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# Deep slate behind, amber in front. The slate is the dark theme's chrome, not black: an icon that
# is black on a dark taskbar is a hole rather than a mark.
$slate = [System.Drawing.Color]::FromArgb(255, 27, 35, 48)
$amber = [System.Drawing.Color]::FromArgb(255, 240, 162, 46)

# The hawk, in a 100x100 box, y downward — a bird seen from above, gliding away from the viewer.
#
# **Curves, and an asymmetric one at that.** The first cut of this mark was a ten-point polygon
# with the wingtips straight out to the sides and sharp points at the head and tail; it drew a
# four-pointed compass star, not a bird. Three things fix it and all three matter: the wings sweep
# *back* so a wingtip points down-and-outward rather than sideways, the head is **rounded** rather
# than pointed, and the tail is **fanned and flat-ended** rather than a fourth spike. What is left
# reads as a raptor at 16 px, which is the only size that was ever in doubt.
#
# Each entry is either a line to a point, or a bezier: the two control points then the endpoint.
# The path is traversed clockwise from the crown.
$hawk = @(
    @{ k = 'bezier'; p = @(58.0, 8.0, 60.0, 18.0, 59.0, 29.0) }    # crown over to the right shoulder
    @{ k = 'bezier'; p = @(71.0, 33.0, 85.0, 46.0, 96.0, 67.0) }   # right wing, leading edge swept back
    @{ k = 'bezier'; p = @(80.0, 65.0, 68.0, 61.0, 60.0, 58.0) }   # right wing, trailing edge inboard
    @{ k = 'line'; p = @(58.0, 72.0) }                             # right flank, down to the tail base
    @{ k = 'bezier'; p = @(62.0, 80.0, 63.0, 88.0, 62.0, 94.0) }   # tail, right edge fanning out
    @{ k = 'bezier'; p = @(56.0, 90.0, 44.0, 90.0, 38.0, 94.0) }   # tail, notched trailing edge
    @{ k = 'bezier'; p = @(37.0, 88.0, 38.0, 80.0, 42.0, 72.0) }   # tail, left edge
    @{ k = 'line'; p = @(40.0, 58.0) }                             # left flank, up to the wing root
    @{ k = 'bezier'; p = @(32.0, 61.0, 20.0, 65.0, 4.0, 67.0) }    # left wing, trailing edge outboard
    @{ k = 'bezier'; p = @(15.0, 46.0, 29.0, 33.0, 41.0, 29.0) }   # left wing, leading edge back inboard
    @{ k = 'bezier'; p = @(40.0, 18.0, 42.0, 8.0, 50.0, 6.0) }     # left of the head, closing at the crown
)
$hawkStart = @(50.0, 6.0)

function New-IconBitmap {
    param([int]$Size)

    $bmp = New-Object System.Drawing.Bitmap($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)

    $s = $Size / 100.0

    # The rounded square. The corner radius rides the size so the silhouette is the same shape at
    # 16 px as at 256; below about 20 px the radius is rounded to whole pixels or the corners fur.
    $radius = [Math]::Max(2.0, [Math]::Round($Size * 0.18))
    $d = $radius * 2.0
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $w = $Size
    $path.AddArc(0, 0, $d, $d, 180, 90)
    $path.AddArc($w - $d, 0, $d, $d, 270, 90)
    $path.AddArc($w - $d, $w - $d, $d, $d, 0, 90)
    $path.AddArc(0, $w - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    $brush = New-Object System.Drawing.SolidBrush($slate)
    $g.FillPath($brush, $path)
    $brush.Dispose()
    $path.Dispose()

    # The hawk, inset so it does not crowd the rounded corners.
    $inset = $Size * 0.07
    $span = $Size - ($inset * 2.0)
    $map = {
        param($x, $y)
        New-Object System.Drawing.PointF(
            [float]($inset + ($x / 100.0) * $span),
            [float]($inset + ($y / 100.0) * $span))
    }

    $bird = New-Object System.Drawing.Drawing2D.GraphicsPath
    $cursor = & $map $hawkStart[0] $hawkStart[1]
    foreach ($seg in $hawk) {
        $p = $seg.p
        if ($seg.k -eq 'line') {
            $to = & $map $p[0] $p[1]
            $bird.AddLine($cursor, $to)
        }
        else {
            $c1 = & $map $p[0] $p[1]
            $c2 = & $map $p[2] $p[3]
            $to = & $map $p[4] $p[5]
            $bird.AddBezier($cursor, $c1, $c2, $to)
        }
        $cursor = $to
    }
    $bird.CloseFigure()

    $birdBrush = New-Object System.Drawing.SolidBrush($amber)
    $g.FillPath($birdBrush, $bird)
    $birdBrush.Dispose()
    $bird.Dispose()

    $g.Dispose()
    return $bmp
}

function Get-PngBytes {
    param([System.Drawing.Bitmap]$Bitmap)
    $ms = New-Object System.IO.MemoryStream
    $Bitmap.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bytes = $ms.ToArray()
    $ms.Dispose()
    return , $bytes
}

# The sizes the shell and the future installer ask for. 16 and 32 are what the window class uses;
# 256 is what Explorer's large views and the installer want.
$sizes = @(16, 20, 24, 32, 48, 64, 128, 256)
$images = @{}

foreach ($size in $sizes) {
    $bmp = New-IconBitmap -Size $size
    $png = Get-PngBytes -Bitmap $bmp
    $images[$size] = $png
    $bmp.Dispose()
    $path = Join-Path $OutDir "icon-$size.png"
    [IO.File]::WriteAllBytes($path, $png)
    "wrote $path ($($png.Length) bytes)"
}

# Assemble the .ico. Every entry is PNG-compressed, which Windows has understood since Vista and
# `SPEC.md` §2.1 scopes this to Windows 10 1809+ — so there is no reason to carry BMP entries and
# their masks. A 256 entry records its width as 0, which is the format's way of saying 256.
$icoPath = Join-Path $OutDir 'tailhawk.ico'
$fs = [IO.File]::Create($icoPath)
$bw = New-Object IO.BinaryWriter($fs)
$bw.Write([UInt16]0)             # reserved
$bw.Write([UInt16]1)             # type: icon
$bw.Write([UInt16]$sizes.Count)

$offset = 6 + (16 * $sizes.Count)
foreach ($size in $sizes) {
    $png = $images[$size]
    $dim = [Byte]0
    if ($size -lt 256) { $dim = [Byte]$size }
    $bw.Write($dim)
    $bw.Write($dim)
    $bw.Write([Byte]0)           # palette entries: none, it is 32-bit
    $bw.Write([Byte]0)           # reserved
    $bw.Write([UInt16]1)         # colour planes
    $bw.Write([UInt16]32)        # bits per pixel
    $bw.Write([UInt32]$png.Length)
    $bw.Write([UInt32]$offset)
    $offset += $png.Length
}
foreach ($size in $sizes) { $bw.Write($images[$size]) }
$bw.Dispose()
$fs.Dispose()
"wrote $icoPath"
