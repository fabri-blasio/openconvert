/**
 * Raster stand-ins for the backend's renderer. **Not shipped.**
 *
 * These produce **real PNG bytes** through a canvas, not an inline SVG, because
 * the point of the exercise is the path the component actually takes: ask the
 * backend for a preview, receive a `data:image/png;base64,…`, hand it to an
 * `<img>`. A drawing pasted into the markup would test none of that.
 *
 * In the desktop build `preview_file` returns the same shape, rendered by a
 * confined worker from the user's actual file.
 */

function canvas(w: number, h: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const ctx = c.getContext("2d");
  if (!ctx) throw new Error("preview: 2d context unavailable");
  return [c, ctx];
}

/** A photograph-shaped image: sky, sun, hills, and a subject to cut out. */
export function photo(w = 960, h = 660, withBackground = true): string {
  const [c, x] = canvas(w, h);

  if (withBackground) {
    const sky = x.createLinearGradient(0, 0, 0, h * 0.78);
    sky.addColorStop(0, "#5b91c4");
    sky.addColorStop(1, "#bcd6e8");
    x.fillStyle = sky;
    x.fillRect(0, 0, w, h);

    // Sun, with a soft bloom so the gradient is visible on a real decode.
    const bloom = x.createRadialGradient(w * 0.8, h * 0.2, 4, w * 0.8, h * 0.2, w * 0.22);
    bloom.addColorStop(0, "rgba(255,240,190,0.95)");
    bloom.addColorStop(1, "rgba(255,240,190,0)");
    x.fillStyle = bloom;
    x.fillRect(0, 0, w, h);

    // Two ridges.
    x.fillStyle = "#6f8f6a";
    x.beginPath();
    x.moveTo(0, h * 0.76);
    x.lineTo(w * 0.28, h * 0.46);
    x.lineTo(w * 0.52, h * 0.76);
    x.closePath();
    x.fill();

    x.fillStyle = "#587a55";
    x.beginPath();
    x.moveTo(w * 0.38, h * 0.76);
    x.lineTo(w * 0.68, h * 0.4);
    x.lineTo(w, h * 0.76);
    x.closePath();
    x.fill();

    x.fillStyle = "#47643f";
    x.fillRect(0, h * 0.76, w, h * 0.24);
  }

  // The subject, drawn in both states — that is what makes the cutout legible.
  x.save();
  x.translate(w / 2, h * 0.9);
  x.fillStyle = "rgba(0,0,0,0.18)";
  x.beginPath();
  x.ellipse(0, 0, w * 0.11, h * 0.02, 0, 0, Math.PI * 2);
  x.fill();
  x.fillStyle = "#c9563f";
  x.beginPath();
  x.moveTo(-w * 0.085, 0);
  x.quadraticCurveTo(0, -h * 0.36, w * 0.085, 0);
  x.closePath();
  x.fill();
  x.fillStyle = "#dcae8a";
  x.beginPath();
  x.arc(0, -h * 0.34, w * 0.055, 0, Math.PI * 2);
  x.fill();
  x.restore();

  return c.toDataURL("image/png");
}

/**
 * Apply the pixel operations the backend applies, to the same data URI.
 *
 * Mirrors `downscale_png`'s op loop in `src-tauri/src/main.rs` — invert leaves
 * alpha alone, greyscale uses the same luma weights — so what the harness shows
 * is what the desktop build renders.
 */
export function applyOps(dataUri: string, ops: string[]): Promise<string> {
  if (ops.length === 0) return Promise.resolve(dataUri);
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const [c, x] = canvas(img.naturalWidth, img.naturalHeight);
      x.drawImage(img, 0, 0);
      const data = x.getImageData(0, 0, c.width, c.height);
      const p = data.data;
      for (const op of ops) {
        if (op === "image-invert") {
          for (let i = 0; i < p.length; i += 4) {
            p[i] = 255 - p[i];
            p[i + 1] = 255 - p[i + 1];
            p[i + 2] = 255 - p[i + 2];
            // Alpha untouched: inverting transparency is not inverting colour.
          }
        } else if (op === "image-greyscale") {
          for (let i = 0; i < p.length; i += 4) {
            const l = Math.round(0.2126 * p[i] + 0.7152 * p[i + 1] + 0.0722 * p[i + 2]);
            p[i] = l;
            p[i + 1] = l;
            p[i + 2] = l;
          }
        }
      }
      x.putImageData(data, 0, 0);
      resolve(c.toDataURL("image/png"));
    };
    img.onerror = () => resolve(dataUri);
    img.src = dataUri;
  });
}

/** One page of a document, with a heading, body lines and a page number. */
export function page(n: number, total: number, w = 760, h = 1074): string {
  const [c, x] = canvas(w, h);

  x.fillStyle = "#ffffff";
  x.fillRect(0, 0, w, h);

  const m = w * 0.12;
  x.fillStyle = "#1d1d1f";
  x.font = `600 ${Math.round(w * 0.045)}px system-ui, sans-serif`;
  x.fillText(n === 1 ? "Service Agreement" : `Section ${n}`, m, m + w * 0.04);

  x.fillStyle = "#8a8a90";
  x.font = `${Math.round(w * 0.022)}px system-ui, sans-serif`;
  x.fillText(`Page ${n} of ${total}`, m, h - m * 0.55);

  // Body: deterministic line lengths so a page looks the same every render.
  let y = m + w * 0.12;
  let seed = n * 977;
  const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  while (y < h - m) {
    const isGap = rand() < 0.12;
    if (isGap) {
      y += w * 0.05;
      continue;
    }
    const len = 0.55 + rand() * 0.45;
    x.fillStyle = "#d5d5da";
    x.fillRect(m, y, (w - m * 2) * len, Math.max(3, w * 0.008));
    y += w * 0.032;
  }

  return c.toDataURL("image/png");
}
