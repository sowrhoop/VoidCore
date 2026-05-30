# Regenerates crates/gui/assets/voidcore-icon.ico from voidcore-icon.png.
# Requires: pip install pillow

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$png = Join-Path $root "crates\gui\assets\voidcore-icon.png"
$ico = Join-Path $root "crates\gui\assets\voidcore-icon.ico"

if (-not (Test-Path $png)) {
    Write-Error "Missing source PNG: $png"
}

python -c @"
from PIL import Image
img = Image.open(r'$png').convert('RGBA')
sizes = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
img.save(r'$ico', format='ICO', sizes=sizes)
print('Wrote', r'$ico')
"@
