import { html, nothing, svg } from "lit";

const WIDTH = 600;
const PAD = { top: 8, right: 8, bottom: 20, left: 52 };

/** A round axis maximum (1, 2 or 5 × 10ⁿ) at or above `value`. */
export function niceMax(value) {
  if (!Number.isFinite(value) || value <= 0) return 1;
  const power = 10 ** Math.floor(Math.log10(value));
  const step = [1, 2, 5, 10].find((factor) => factor * power >= value);
  return step * power;
}

/** Bytes per second as a short bit-rate label (network convention). */
export function formatRate(bytesPerSecond) {
  const bits = Math.max(0, Number(bytesPerSecond) || 0) * 8;
  const units = ["bps", "Kbps", "Mbps", "Gbps"];
  let value = bits;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const digits = value >= 100 || unit === 0 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

/** Bytes per second as KB/s, MB/s (for users who think in file sizes). */
export function formatByteRate(bytesPerSecond) {
  let value = Math.max(0, Number(bytesPerSecond) || 0);
  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/**
 * Stacked bars, one per item: `items` are `{ label, values: [a, b] }` drawn
 * with the series-1 and series-2 colors. Labels thin out when there are many.
 */
export function barChart({ items, height = 160, formatValue, ariaLabel, legend = [] }) {
  if (!items.length) return nothing;
  const plot = { width: WIDTH - PAD.left - PAD.right, height: height - PAD.top - PAD.bottom };
  const top = niceMax(Math.max(...items.map((item) => item.values.reduce((sum, value) => sum + (value || 0), 0))));
  const slot = plot.width / items.length;
  const bar = Math.max(2, Math.min(28, slot * 0.7));
  const every = Math.ceil(items.length / 10);
  const baseline = PAD.top + plot.height;
  return html`<figure class="line-chart bar-chart">
    <svg viewBox="0 0 ${WIDTH} ${height}" role="img" aria-label=${ariaLabel}>
      ${[0, 0.5, 1].map((share) => {
        const y = baseline - share * plot.height;
        return svg`<line class="grid" x1=${PAD.left} x2=${WIDTH - PAD.right} y1=${y} y2=${y}></line>
          <text class="axis" x=${PAD.left - 6} y=${y + 4} text-anchor="end">${formatValue(top * share)}</text>`;
      })}
      ${items.map((item, index) => {
        const x = PAD.left + index * slot + (slot - bar) / 2;
        let y = baseline;
        return svg`<g>
          <title>${item.label}: ${item.values.map((value) => formatValue(value || 0)).join(" / ")}</title>
          ${item.values.map((value, part) => {
            const h = ((value || 0) / top) * plot.height;
            y -= h;
            return svg`<rect class="bar series-${part + 1}" x=${x.toFixed(1)} y=${y.toFixed(1)} width=${bar.toFixed(1)} height=${Math.max(0, h).toFixed(1)}></rect>`;
          })}
          ${index % every === 0 ? svg`<text class="axis" x=${(x + bar / 2).toFixed(1)} y=${height - 4} text-anchor="middle">${item.label}</text>` : nothing}
        </g>`;
      })}
    </svg>
    ${legend.length
      ? html`<figcaption class="chart-legend">
          ${legend.map((label, index) => html`<span><i class="key series-${index + 1}"></i>${label}</span>`)}
        </figcaption>`
      : nothing}
  </figure>`;
}

/** SVG path for points `[time, value]` scaled into the plot box. */
export function linePath(points, { start, end, max, width, height }) {
  const span = Math.max(1, end - start);
  let path = "";
  let pen = false;
  for (const [time, value] of points) {
    if (value == null) {
      pen = false;
      continue;
    }
    const x = PAD.left + ((time - start) / span) * width;
    const y = PAD.top + height - (Math.min(value, max) / max) * height;
    path += `${pen ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`;
    pen = true;
  }
  return path;
}

const nearestIndex = (points, time) => {
  let best = -1;
  let distance = Infinity;
  points.forEach(([t], index) => {
    const d = Math.abs(t - time);
    if (d < distance) {
      distance = d;
      best = index;
    }
  });
  return best;
};

/**
 * A small line chart with a hairline grid, a crosshair tooltip and one
 * y-axis. `series` items: `{ key, label, points: [[time, value]], tone }`.
 */
export function lineChart({ id, series, height = 160, max, formatValue, formatTime, hover, onHover, ariaLabel, area = true }) {
  const all = series.flatMap((item) => item.points);
  if (all.length < 2) return nothing;
  const start = Math.min(...all.map(([time]) => time));
  const end = Math.max(...all.map(([time]) => time));
  const top = max ?? niceMax(Math.max(...all.map(([, value]) => value ?? 0)));
  const plot = { start, end, max: top, width: WIDTH - PAD.left - PAD.right, height: height - PAD.top - PAD.bottom };
  const baseline = PAD.top + plot.height;
  const ticks = [0, 0.5, 1].map((share) => ({ y: baseline - share * plot.height, label: formatValue(top * share) }));
  const reference = series[0].points;
  const index = hover?.chart === id ? hover.index : -1;
  const active = reference[index];
  const crossX = active ? PAD.left + ((active[0] - start) / Math.max(1, end - start)) * plot.width : null;
  const move = (event) => {
    const box = event.currentTarget.getBoundingClientRect();
    const x = ((event.clientX - box.left) / box.width) * WIDTH;
    const time = start + ((x - PAD.left) / plot.width) * (end - start);
    onHover({ chart: id, index: nearestIndex(reference, time) });
  };
  return html`<figure class="line-chart">
    <svg
      viewBox="0 0 ${WIDTH} ${height}"
      role="img"
      aria-label=${ariaLabel}
      @pointermove=${move}
      @pointerleave=${() => onHover(null)}
    >
      ${ticks.map(
        (tick) => svg`<line class="grid" x1=${PAD.left} x2=${WIDTH - PAD.right} y1=${tick.y} y2=${tick.y}></line>
          <text class="axis" x=${PAD.left - 6} y=${tick.y + 4} text-anchor="end">${tick.label}</text>`,
      )}
      <text class="axis" x=${PAD.left} y=${height - 4}>${formatTime(start)}</text>
      <text class="axis" x=${WIDTH - PAD.right} y=${height - 4} text-anchor="end">${formatTime(end)}</text>
      ${series.map((item) => {
        const path = linePath(item.points, plot);
        return svg`${area && path
          ? svg`<path class="area ${item.tone}" d=${`${path}L${(PAD.left + plot.width).toFixed(1)},${baseline}L${PAD.left},${baseline}Z`}></path>`
          : nothing}<path class="line ${item.tone}" d=${path}></path>`;
      })}
      ${crossX != null ? svg`<line class="crosshair" x1=${crossX} x2=${crossX} y1=${PAD.top} y2=${baseline}></line>` : nothing}
    </svg>
    ${series.length > 1
      ? html`<figcaption class="chart-legend">
          ${series.map((item) => html`<span><i class="key ${item.tone}"></i>${item.label}</span>`)}
        </figcaption>`
      : nothing}
    ${active
      ? html`<div class="chart-tooltip ${crossX > WIDTH / 2 ? "at-start" : "at-end"}">
          <strong>${formatTime(active[0])}</strong>
          ${series.map((item) => {
            const point = item.points[index];
            return html`<span><i class="key ${item.tone}"></i>${item.label}: ${point?.[1] == null ? "—" : formatValue(point[1])}</span>`;
          })}
        </div>`
      : nothing}
  </figure>`;
}
