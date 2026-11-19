"""Charts the JSON that `cargo run --release --example perf_survey` prints.

    python3 scripts/plot_perf.py mutil-engine/target/perf_data.json

The PNGs are written next to the data file. Needs matplotlib and numpy.
"""

import json
import os
import sys

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

if len(sys.argv) != 2:
    sys.exit(__doc__)
DATA_PATH = sys.argv[1]
ART_DIR = os.path.dirname(os.path.abspath(DATA_PATH))

with open(DATA_PATH) as f:
    data = json.load(f)

plt.rcParams.update({
    "font.family": "sans-serif",
    "font.size": 10,
    "axes.edgecolor": "#cccccc",
    "axes.linewidth": 0.8,
    "grid.color": "#e5e5e5",
    "grid.linestyle": "--",
    "grid.linewidth": 0.6,
})

# ==============================================================================
# Chart 1: End-to-End Pipeline Stage Breakdown (2000 chars)
# ==============================================================================
wl = data["workloads"]
labels_map = {
    "latin_plain_harfrust": "Latin Plain (HarfRust)",
    "latin_plain_stub": "Latin Plain (Stub Shaper)",
    "latin_justified": "Latin Justified (HarfRust)",
    "cjk_mixed_harfrust": "CJK + Latin Mixed (HarfRust)",
    "cjk_mixed_stub": "CJK + Latin Mixed (Stub)",
    "arabic_bidi_harfrust": "Arabic Bidi RTL (HarfRust)",
    "devanagari_harfrust": "Devanagari Complex (HarfRust)",
    "rich_spans_harfrust": "Rich Spans 50+50 (HarfRust)",
    "narrow_emergency_harfrust": "Narrow 48px (HarfRust)",
}

names = [labels_map.get(w["name"], w["name"]) for w in wl][::-1]
compile_ms = np.array([w["compile_us"] / 1000.0 for w in wl][::-1])
val_in_ms = np.array([w["val_in_us"] / 1000.0 for w in wl][::-1])
layout_only_ms = np.array([max(0.0, w["layout_total_us"] - w["val_in_us"]) / 1000.0 for w in wl][::-1])
val_out_ms = np.array([w["val_out_us"] / 1000.0 for w in wl][::-1])

fig, ax = plt.subplots(figsize=(10.5, 5.2), dpi=160)
y = np.arange(len(names))
h = 0.62

p1 = ax.barh(y, compile_ms, h, label="Compile (Frontend + Shaping)", color="#3b82f6")
p2 = ax.barh(y, val_in_ms, h, left=compile_ms, label="Input Validation Gate", color="#f59e0b")
p3 = ax.barh(y, layout_only_ms, h, left=compile_ms + val_in_ms, label="Layout (Breaking + Placement)", color="#10b981")
p4 = ax.barh(y, val_out_ms, h, left=compile_ms + val_in_ms + layout_only_ms, label="Output Validation Gate", color="#8b5cf6")

totals = compile_ms + val_in_ms + layout_only_ms + val_out_ms
for i, tot in enumerate(totals):
    ax.text(tot + 0.05, y[i], f"{tot:.2f} ms", va="center", ha="left", fontsize=9, fontweight="bold", color="#333333")

ax.set_yticks(y)
ax.set_yticklabels(names)
ax.set_xlabel("Median Latency (ms) — 2,000 characters")
ax.set_title("mutil-engine End-to-End Pipeline Latency Breakdown (Release)", fontweight="bold", pad=12)
ax.set_xlim(0, max(totals) * 1.15)
ax.grid(axis="x", zorder=0)
ax.set_axisbelow(True)
ax.legend(loc="lower right", frameon=True, facecolor="white", framealpha=0.95)
plt.tight_layout()
plt.savefig(os.path.join(ART_DIR, "stage_breakdown.png"))
plt.close()

# ==============================================================================
# Chart 2: Heap Allocations Breakdown (Count & MB)
# ==============================================================================
fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(11.5, 4.8), dpi=160)

comp_allocs = [w["compile_allocs"] for w in wl][::-1]
lay_allocs = [w["layout_allocs"] for w in wl][::-1]

y = np.arange(len(names))
h = 0.36
ax1.barh(y + h/2, comp_allocs, h, label="Compile Allocs", color="#ef4444")
ax1.barh(y - h/2, lay_allocs, h, label="Layout Allocs", color="#06b6d4")
ax1.set_yticks(y)
ax1.set_yticklabels(names, fontsize=8.5)
ax1.set_xlabel("Heap Allocations per Paragraph")
ax1.set_title("Heap Allocation Count (2,000 chars)", fontweight="bold")
ax1.grid(axis="x", zorder=0)
ax1.set_axisbelow(True)
ax1.legend(loc="lower right", fontsize=8.5)

comp_mb = [w["compile_bytes"] / (1024 * 1024) for w in wl][::-1]
lay_mb = [w["layout_bytes"] / (1024 * 1024) for w in wl][::-1]
ax2.barh(y + h/2, comp_mb, h, label="Compile Bytes (MB)", color="#f97316")
ax2.barh(y - h/2, lay_mb, h, label="Layout Bytes (MB)", color="#14b8a6")
ax2.set_yticks(y)
ax2.set_yticklabels([])
ax2.set_xlabel("Total Bytes Allocated (MB)")
ax2.set_title("Heap Allocation Volume (2,000 chars)", fontweight="bold")
ax2.grid(axis="x", zorder=0)
ax2.set_axisbelow(True)
ax2.legend(loc="lower right", fontsize=8.5)

plt.tight_layout()
plt.savefig(os.path.join(ART_DIR, "allocations_breakdown.png"))
plt.close()

# ==============================================================================
# Chart 3: Scaling Curves (O(N) Wrapped vs O(N^2) Unwrapped + Interaction)
# ==============================================================================
sc = data["scaling"]
wl_wide = data["wide_line"]

latin_sc = [x for x in sc if x["name"].startswith("scale_latin_")]
ns = np.array([x["chars"] for x in latin_sc])
c_us = np.array([x["compile_us"] for x in latin_sc])
l_us = np.array([x["layout_total_us"] for x in latin_sc])
vo_us = np.array([x["val_out_us"] for x in latin_sc])
vi_us = np.array([x["val_in_us"] for x in latin_sc])

wide_ns = np.array([x["chars"] for x in wl_wide])
wide_l_us = np.array([x["layout_total_us"] for x in wl_wide])

hit_ns = np.array([x["hit_test_ns"] for x in latin_sc])
caret_ns = np.array([x["caret_geom_ns"] for x in latin_sc])

fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(11.5, 4.6), dpi=160)

# Left: Pipeline & Wide-line O(N^2) scaling
ax1.loglog(ns, c_us, "o-", label="Compile (HarfRust, ~O(N))", color="#3b82f6", linewidth=2)
ax1.loglog(ns, l_us, "s-", label="Layout Wrapped 480px (~O(N))", color="#10b981", linewidth=2)
ax1.loglog(wide_ns, wide_l_us, "^--", label="Layout Single-Line Unwrapped", color="#ef4444", linewidth=2.2, markersize=7)
ax1.loglog(ns, vo_us, "d:", label="Output Validation (~O(N))", color="#8b5cf6", linewidth=1.6)
ax1.loglog(ns, vi_us, "x:", label="Input Validation (~O(N))", color="#f59e0b", linewidth=1.6)

# Annotate the O(N^2) crossover
ax1.annotate(f"{wide_l_us[-1] / 1000:.2f} ms unwrapped\n@ {wide_ns[-1]:,} chars",
             xy=(wide_ns[-1], wide_l_us[-1]), xytext=(450, 2200),
             arrowprops=dict(arrowstyle="->", color="#ef4444", lw=1.2),
             fontsize=8.5, fontweight="bold", color="#b91c1c")

ax1.set_xlabel("Paragraph Length (characters, log scale)")
ax1.set_ylabel("Median Latency (µs, log scale)")
ax1.set_title("Pipeline Scaling: Wrapped vs Wide Line", fontweight="bold")
ax1.grid(True, which="both", alpha=0.5)
ax1.legend(loc="lower right", fontsize=8)

# Right: Interaction query scaling
ax2.loglog(ns, caret_ns, "o-", label="caret_geometry()", color="#f97316", linewidth=2.2)
ax2.loglog(ns, hit_ns, "s-", label="hit_test() (bisection)", color="#06b6d4", linewidth=2.2)
ax2.annotate(f"{caret_ns[-1]:,.0f} ns @ {ns[-1]:,} chars",
             xy=(ns[-1], caret_ns[-1]), xytext=(800, 1200),
             arrowprops=dict(arrowstyle="->", color="#f97316", lw=1.2),
             fontsize=8.5, fontweight="bold", color="#c2410c")
ax2.annotate(f"{hit_ns[-1]:,.1f} ns @ {ns[-1]:,} chars\n({caret_ns[-1] / hit_ns[-1]:.0f}× faster than caret_geometry)",
             xy=(ns[-1], hit_ns[-1]), xytext=(600, 45),
             arrowprops=dict(arrowstyle="->", color="#06b6d4", lw=1.2),
             fontsize=8.5, fontweight="bold", color="#0e7490")

ax2.set_xlabel("Paragraph Length (characters, log scale)")
ax2.set_ylabel("Latency per Query (ns, log scale)")
ax2.set_title("Interaction Query Scaling: hit_test vs caret_geometry", fontweight="bold")
ax2.grid(True, which="both", alpha=0.5)
ax2.legend(loc="upper left", fontsize=8.5)

plt.tight_layout()
plt.savefig(os.path.join(ART_DIR, "scaling_curves.png"))
plt.close()
print("Charts generated successfully.")
