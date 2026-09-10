"""Build the file-opening icon around the traced Mole 04 foreground."""

from pathlib import Path
from xml.etree import ElementTree as ET


HERE = Path(__file__).resolve().parent
SOURCE = HERE / "openconvert-mole-monoline-traced.svg"
OUTPUT = HERE / "openconvert-mole-file-opening.svg"
NS = {"svg": "http://www.w3.org/2000/svg"}

root = ET.parse(SOURCE).getroot()
paths = root.findall("svg:path", NS)
ink = next(path.attrib["d"] for path in paths if path.attrib.get("fill") == "#1D1D1F")
brand = next(path.attrib["d"] for path in paths if path.attrib.get("fill") == "#2F6F6A")
paper = next(path.attrib["d"] for path in paths if path.attrib.get("fill") == "#FAFAFA")

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" role="img" aria-labelledby="title desc">
  <title id="title">OpenConvert mole emerging from a file</title>
  <desc id="desc">The traced monoline mole peeks through a document-shaped opening with a folded corner.</desc>
  <defs>
    <!-- This isolates the original animal from the concentric tunnel lines. -->
    <clipPath id="mole-foreground">
      <path d="M270 902V706c25-141 94-264 212-316 137-62 245 10 283 147l38 30h58v84c-6 39-24 72-55 96-17 83-72 143-159 167Z"/>
    </clipPath>
  </defs>

  <!-- Platform-safe square. -->
  <rect x="24" y="24" width="976" height="976" rx="232" fill="#FAFAFA"/>

  <!-- A document is now the opening: one object, one colour, clear depth. -->
  <path d="M288 108h356l174 174v516c0 45-37 82-82 82H288c-45 0-82-37-82-82V190c0-45 37-82 82-82Z"
        fill="#FFFFFF" stroke="#2F6F6A" stroke-width="42" stroke-linejoin="round"/>
  <path d="M644 108v174h174" fill="none" stroke="#2F6F6A" stroke-width="42" stroke-linecap="round" stroke-linejoin="round"/>

  <!-- Exact traced Mole 04 foreground. -->
  <g clip-path="url(#mole-foreground)">
    <path d="{ink}" fill="#1D1D1F" fill-rule="evenodd"/>
    <path d="{brand}" fill="#2F6F6A" fill-rule="evenodd"/>
    <path d="{paper}" fill="#FAFAFA" fill-rule="evenodd"/>
  </g>

  <!-- Preserve the traced nose silhouette, replace its single central void,
       then add the two nostrils a mole actually has. -->
  <ellipse cx="720" cy="638" rx="23" ry="17" transform="rotate(-8 720 638)" fill="#2F6F6A"/>
  <ellipse cx="704" cy="637" rx="8" ry="6" transform="rotate(-12 704 637)" fill="#1D1D1F"/>
  <ellipse cx="730" cy="631" rx="8" ry="6" transform="rotate(-12 730 631)" fill="#1D1D1F"/>
</svg>
'''

OUTPUT.write_text(svg, encoding="utf-8")
print(f"wrote {OUTPUT}")
