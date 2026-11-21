// Canvas drawing for the page. It only paints what the engine already
// computed (spec HST-5: geometry is never adjusted at draw time) and reads
// the DOM only through the canvas it is given.

/** Canvas geometry the layout and the hit test must agree on. */
export const PAGE = Object.freeze({ pad: 32, paragraphGap: 14 });

/**
 * Binds drawing to `canvas`. `drawPage` paints a laid-out view; `drawPlaceholder`
 * paints a one-line message. Glyph outlines are cached per renderer.
 */
export function createRenderer(canvas) {
  const ctx = canvas.getContext('2d');
  const glyphs = new Map(); // "font:gid" -> Path2D | null

  function sizeCanvas(cssWidth, cssHeight) {
    const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
    canvas.width = Math.round(cssWidth * dpr);
    canvas.height = Math.round(cssHeight * dpr);
    canvas.style.width = `${cssWidth}px`;
    canvas.style.height = `${cssHeight}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssWidth, cssHeight);
    ctx.fillStyle = '#fffdf7';
    ctx.fillRect(0, 0, cssWidth, cssHeight);
  }

  /**
   * `scene`: { engine, fonts, selBounds, focus, imeRange, guides, selectionHeight }.
   * An empty view only sizes the canvas.
   */
  function drawPage(view, scene) {
    const { results } = view;
    const last = results.at(-1);
    sizeCanvas(
      view.width + PAGE.pad * 2,
      Math.max((last ? last.y + last.blockSize : PAGE.pad) + PAGE.pad, 200),
    );
    for (const paragraph of results) {
      ctx.save();
      ctx.translate(PAGE.pad, paragraph.y);
      drawLineGuides(paragraph, scene.guides);
      drawSpanFragments(paragraph);
      if (scene.selBounds) {
        drawSelection(scene, paragraph.id);
      }
      drawGlyphs(scene, paragraph);
      drawDecorations(paragraph);
      drawCaretStops(paragraph, scene.guides);
      if (scene.imeRange && scene.imeRange.paraId === paragraph.id) {
        drawImeOverlay(scene.engine, scene.imeRange);
      }
      if (scene.focus && scene.focus.paraId === paragraph.id) {
        drawActiveCaret(scene.engine, scene.focus);
      }
      ctx.restore();
    }
  }

  function colorForByte(paintSpans, byteOffset) {
    if (byteOffset === null || byteOffset === undefined || !paintSpans?.length) {
      return '#1b1a17';
    }
    for (const span of paintSpans) {
      if (byteOffset >= span.start && byteOffset < span.end) {
        return span.color;
      }
    }
    return '#1b1a17';
  }

  function drawSpanFragments(paragraph) {
    const styles = new Map((paragraph.spanStyles ?? []).map((s) => [s.spanId, s.kind]));
    for (const line of paragraph.lines) {
      for (const frag of line.spanFragments ?? []) {
        if (frag.width <= 0) {
          continue;
        }
        const kind = styles.get(frag.spanId) ?? 'mark';
        const top = line.baseline - frag.ascent;
        const height = frag.ascent + frag.descent;
        if (height <= 0) {
          continue;
        }
        const [leftEdge, rightEdge] = frag.edges ?? ['logicalEnd', 'logicalEnd'];
        const closedLeft = leftEdge === 'logicalEnd' || leftEdge === 'clonedWrap';
        const closedRight = rightEdge === 'logicalEnd' || rightEdge === 'clonedWrap';
        const radius = kind === 'pill' ? Math.min(6, height / 2) : Math.min(3, height / 2);
        const rL = closedLeft ? radius : 0;
        const rR = closedRight ? radius : 0;

        ctx.save();
        traceBoxPath(ctx, frag.x, top, frag.width, height, rL, rR);
        if (kind === 'ribbon') {
          const total = frag.totalSpanAdvance > 0 ? frag.totalSpanAdvance : frag.width;
          const t0 = Math.max(0, Math.min(1, frag.ribbonOffset / total));
          const t1 = Math.max(t0, Math.min(1, (frag.ribbonOffset + frag.width) / total));
          const grad = ctx.createLinearGradient(frag.x, 0, frag.x + frag.width, 0);
          grad.addColorStop(0, ribbonColor(t0));
          grad.addColorStop(1, ribbonColor(t1));
          ctx.fillStyle = grad;
          ctx.fill();
          strokeOpenEdges(ctx, frag.x, top, frag.width, height, closedLeft, closedRight, 'rgba(14, 116, 144, 0.45)');
        } else if (kind === 'pill') {
          ctx.fillStyle = 'rgba(79, 112, 189, 0.14)';
          ctx.fill();
          strokeOpenEdges(ctx, frag.x, top, frag.width, height, closedLeft, closedRight, 'rgba(79, 112, 189, 0.55)');
        } else {
          ctx.fillStyle = 'rgba(250, 204, 21, 0.34)';
          ctx.fill();
        }
        ctx.restore();
      }
    }
  }

  function drawDecorations(paragraph) {
    const paintSpans = paragraph.paintSpans;
    for (const line of paragraph.lines) {
      for (const dec of line.decorations ?? []) {
        if (dec.width <= 0) {
          continue;
        }
        const y = line.baseline - dec.yOffset;
        const thickness = Math.max(1, dec.thickness);
        ctx.fillStyle = colorForByte(paintSpans, dec.byteRange?.[0]);
        ctx.fillRect(dec.x, y - thickness / 2, dec.width, thickness);
      }
    }
  }

  function drawGlyphs({ engine, fonts }, paragraph) {
    const paintSpans = paragraph.paintSpans;
    const byColor = new Map();
    for (const line of paragraph.lines) {
      for (const run of line.runs) {
        const info = fonts.get(run.font);
        if (!info) {
          continue;
        }
        // Outline paths are in font units; the run's size over the face's
        // units-per-em is the only scale applied (spec HST-5: geometry is
        // never adjusted at draw time).
        const scale = run.size / info.upem;
        for (const [glyphId, x, originY, byteStart] of run.glyphs) {
          const outline = glyphPath(engine, run.font, glyphId);
          if (!outline) {
            continue;
          }
          const color = colorForByte(paintSpans, byteStart);
          let path = byColor.get(color);
          if (!path) {
            path = new Path2D();
            byColor.set(color, path);
          }
          path.addPath(outline, new DOMMatrix([scale, 0, 0, scale, x, originY]));
        }
      }
    }
    for (const [color, path] of byColor) {
      ctx.fillStyle = color;
      ctx.fill(path);
    }
  }

  function glyphPath(engine, fontId, glyphId) {
    const key = `${fontId}:${glyphId}`;
    if (glyphs.has(key)) {
      return glyphs.get(key);
    }
    const data = engine.outline(fontId, glyphId);
    const path = data ? new Path2D(data) : null;
    glyphs.set(key, path);
    return path;
  }

  function drawLineGuides(paragraph, guides) {
    if (!guides.boxes && !guides.baselines) {
      return;
    }
    for (const line of paragraph.lines) {
      if (guides.boxes) {
        // The slot's available interval, dashed: a hanging mark protrudes
        // past it, while the solid content box below stops at the body.
        ctx.save();
        ctx.setLineDash([2, 3]);
        ctx.strokeStyle = 'rgba(79, 112, 189, 0.45)';
        ctx.lineWidth = 1;
        for (const [start, end] of line.columns ?? []) {
          if (end !== null) {
            strokeInside(ctx, start, end, line);
          }
        }
        ctx.restore();
        // Each hanging glyph's advance, in amber.
        for (const [start, end] of line.hangs ?? []) {
          ctx.fillStyle = 'rgba(214, 140, 30, 0.22)';
          ctx.fillRect(start, line.top, end - start, line.height);
          ctx.strokeStyle = 'rgba(214, 140, 30, 0.8)';
          ctx.lineWidth = 1;
          strokeInside(ctx, start, end, line);
        }
      }
      for (const [start, end] of line.slots) {
        if (guides.boxes) {
          ctx.fillStyle = 'rgba(79, 112, 189, 0.06)';
          ctx.fillRect(start, line.top, end - start, line.height);
          ctx.strokeStyle = 'rgba(79, 112, 189, 0.35)';
          ctx.lineWidth = 1;
          strokeInside(ctx, start, end, line);
        }
        if (guides.baselines) {
          ctx.save();
          ctx.setLineDash([3, 3]);
          ctx.strokeStyle = 'rgba(79, 112, 189, 0.5)';
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(start, Math.round(line.baseline) + 0.5);
          ctx.lineTo(end, Math.round(line.baseline) + 0.5);
          ctx.stroke();
          ctx.restore();
        }
      }
    }
  }

  function drawCaretStops(paragraph, guides) {
    if (!guides.carets) {
      return;
    }
    ctx.strokeStyle = 'rgba(27, 122, 90, 0.55)';
    ctx.lineWidth = 1;
    for (const line of paragraph.lines) {
      for (const x of line.carets) {
        ctx.beginPath();
        ctx.moveTo(x + 0.5, line.baseline - 4);
        ctx.lineTo(x + 0.5, line.baseline + 4);
        ctx.stroke();
      }
    }
  }

  function drawSelection({ engine, selBounds, selectionHeight }, paraId) {
    if (paraId < selBounds.start.paraId || paraId > selBounds.end.paraId) {
      return;
    }
    const lo = paraId === selBounds.start.paraId ? selBounds.start.byteOffset : 0;
    const hi = paraId === selBounds.end.paraId ? selBounds.end.byteOffset : 0x7fffffff;
    if (lo >= hi) {
      return;
    }
    const rects = engine.selectionGeometry(paraId, lo, hi, selectionHeight);
    if (!rects || rects.length === 0) {
      return;
    }
    ctx.fillStyle = 'rgba(79, 112, 189, 0.24)';
    for (const r of rects) {
      ctx.fillRect(r.x, r.y, r.w, r.h);
    }
  }

  function drawActiveCaret(engine, pos) {
    const caret = engine.caretGeometry(pos.paraId, pos.byteOffset, pos.affinity);
    if (!caret) {
      return;
    }
    const focused = canvas.ownerDocument?.activeElement === canvas;
    ctx.save();
    const px = Math.round(caret.primary.x) + 0.5;
    ctx.strokeStyle = focused ? '#2563eb' : '#1b1a17';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(px, caret.primary.y);
    ctx.lineTo(px, caret.primary.y + caret.primary.h);
    ctx.stroke();

    if (caret.secondary) {
      const sx = Math.round(caret.secondary.x) + 0.5;
      const top = caret.secondary.y + caret.secondary.h * 0.45;
      const bottom = caret.secondary.y + caret.secondary.h;
      ctx.strokeStyle = '#d97706';
      ctx.lineWidth = 1.5;
      ctx.setLineDash([3, 2]);
      ctx.beginPath();
      ctx.moveTo(sx, top);
      ctx.lineTo(sx, bottom);
      ctx.stroke();
    }
    ctx.restore();
  }

  function drawImeOverlay(engine, { paraId, startByte, endByte }) {
    const rects = engine.rangeRects(paraId, startByte, endByte);
    if (!rects || rects.length === 0) {
      return;
    }
    ctx.save();
    ctx.fillStyle = 'rgba(217, 119, 6, 0.12)';
    ctx.strokeStyle = '#d97706';
    ctx.lineWidth = 2;
    ctx.setLineDash([4, 2]);
    for (const r of rects) {
      ctx.fillRect(r.x, r.y, r.w, r.h);
      const uy = Math.round(r.y + r.h - 1) + 0.5;
      ctx.beginPath();
      ctx.moveTo(r.x, uy);
      ctx.lineTo(r.x + r.w, uy);
      ctx.stroke();
    }
    // Simulated IME candidate window anchor at the bottom-left of the first composing rect.
    const anchor = rects[0];
    const ax = Math.round(anchor.x) + 0.5;
    const ay = Math.round(anchor.y + anchor.h) + 0.5;
    ctx.setLineDash([]);
    ctx.fillStyle = '#d97706';
    ctx.beginPath();
    ctx.moveTo(ax, ay);
    ctx.lineTo(ax - 4, ay + 6);
    ctx.lineTo(ax + 4, ay + 6);
    ctx.closePath();
    ctx.fill();

    ctx.font = '11px ui-sans-serif, system-ui, sans-serif';
    const label = 'IME anchor';
    const tw = ctx.measureText(label).width;
    ctx.fillStyle = '#1b1a17';
    ctx.fillRect(ax - 4, ay + 6, tw + 10, 18);
    ctx.fillStyle = '#fffdf7';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(label, ax + 1, ay + 15);
    ctx.restore();
  }

  function drawPlaceholder(width, message) {
    const cssWidth = width + PAGE.pad * 2;
    sizeCanvas(cssWidth, 200);
    ctx.fillStyle = '#8a8378';
    ctx.font = '14px ui-sans-serif, system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(message, cssWidth / 2, 100, width);
  }

  return { drawPage, drawPlaceholder };
}

/**
 * Strokes a one-pixel outline just inside `[start, end)` over the line box.
 * An interval narrower than the line width is drawn as a hairline, never with
 * a negative width.
 */
function strokeInside(ctx, start, end, line) {
  ctx.strokeRect(
    start + 0.5,
    line.top + 0.5,
    Math.max(0, end - start - 1),
    Math.max(0, line.height - 1),
  );
}

function traceBoxPath(ctx, x, y, w, h, rLeft, rRight) {
  const rl = Math.min(rLeft, w / 2, h / 2);
  const rr = Math.min(rRight, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rl, y);
  ctx.lineTo(x + w - rr, y);
  if (rr > 0) ctx.quadraticCurveTo(x + w, y, x + w, y + rr);
  ctx.lineTo(x + w, y + h - rr);
  if (rr > 0) ctx.quadraticCurveTo(x + w, y + h, x + w - rr, y + h);
  ctx.lineTo(x + rl, y + h);
  if (rl > 0) ctx.quadraticCurveTo(x, y + h, x, y + h - rl);
  ctx.lineTo(x, y + rl);
  if (rl > 0) ctx.quadraticCurveTo(x, y, x + rl, y);
  ctx.closePath();
}

function strokeOpenEdges(ctx, x, y, w, h, closedLeft, closedRight, strokeStyle) {
  ctx.strokeStyle = strokeStyle;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(x, y + 0.5);
  ctx.lineTo(x + w, y + 0.5);
  ctx.moveTo(x, y + h - 0.5);
  ctx.lineTo(x + w, y + h - 0.5);
  if (closedLeft) {
    ctx.moveTo(x + 0.5, y);
    ctx.lineTo(x + 0.5, y + h);
  }
  if (closedRight) {
    ctx.moveTo(x + w - 0.5, y);
    ctx.lineTo(x + w - 0.5, y + h);
  }
  ctx.stroke();
}

function ribbonColor(t) {
  const hue = 195 + t * 130;
  return `hsla(${hue.toFixed(1)}, 78%, 78%, 0.42)`;
}
