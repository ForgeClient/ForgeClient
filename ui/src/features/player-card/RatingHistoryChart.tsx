import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import type { RatingHistoryPoint } from "../../ipc/bindings";
import { formatDate, formatDateTime } from "../../shared/dates";
import { formatNumber } from "../../i18n";
import { useTranslation } from "../../i18n/useTranslation";

interface ChartPoint {
  timestamp: number;
  rating: number;
  x: number;
  y: number;
}

interface RatingHistoryChartProps {
  points: RatingHistoryPoint[];
  maximum: RatingHistoryPoint | null;
  showMaximum: boolean;
}

/// The SVG is rendered at its real pixel size rather than scaled from a fixed
/// viewBox. A fixed viewBox with `preserveAspectRatio="none"` stretched every
/// axis label horizontally by whatever the container/viewBox ratio happened to
/// be, which is why the old chart's text looked subtly wide.
const HEIGHT = 300;
const LEFT = 52;
const RIGHT = 16;
const TOP = 16;
const BOTTOM = 32;
const FALLBACK_WIDTH = 900;

const MAX_RENDERED_POINTS = 1_800;

/** Round tick steps a reader can do arithmetic with. */
const NICE_STEPS = [5, 10, 20, 25, 50, 100, 200, 250, 500, 1_000];

function niceStep(raw: number): number {
  return NICE_STEPS.find((step) => step >= raw) ?? NICE_STEPS[NICE_STEPS.length - 1];
}

/** `Math.min(...points)` throws above ~100k arguments, and a complete history
 *  is six figures of games. Every aggregate here is a loop for that reason. */
function extent(values: number[]): { min: number; max: number } {
  let min = values[0];
  let max = values[0];
  for (const value of values) {
    if (value < min) min = value;
    if (value > max) max = value;
  }
  return { min, max };
}

function downsample(points: Array<{ timestamp: number; rating: number }>) {
  if (points.length <= MAX_RENDERED_POINTS) return points;
  const bucketSize = Math.ceil(points.length / (MAX_RENDERED_POINTS / 2));
  const sampled = [points[0]];
  for (let start = 1; start < points.length - 1; start += bucketSize) {
    const bucket = points.slice(start, Math.min(points.length - 1, start + bucketSize));
    let minimum = bucket[0];
    let maximum = bucket[0];
    for (const point of bucket) {
      if (point.rating < minimum.rating) minimum = point;
      if (point.rating > maximum.rating) maximum = point;
    }
    if (minimum.timestamp < maximum.timestamp) sampled.push(minimum, maximum);
    else if (minimum !== maximum) sampled.push(maximum, minimum);
    else sampled.push(minimum);
  }
  sampled.push(points[points.length - 1]);
  return sampled;
}

/**
 * Width of the plot box, so the SVG can be drawn in real pixels.
 *
 * Measured directly on mount and *then* observed. The one-shot measurement is
 * not redundant: it is what makes the chart correct in any environment where
 * `ResizeObserver` does not deliver an initial callback, and it means the
 * first paint is already the right size rather than a fallback that resizes.
 */
function useMeasuredWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(FALLBACK_WIDTH);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = (next: number) => {
      if (next > 0) setWidth(Math.round(next));
    };
    measure(element.clientWidth - horizontalPadding(element));
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => measure(entry.contentRect.width));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return [ref, width] as const;
}

/** `clientWidth` includes padding; `contentRect` does not. Match the observer. */
function horizontalPadding(element: HTMLElement): number {
  const style = getComputedStyle(element);
  return parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
}

export function RatingHistoryChart({ points, maximum, showMaximum }: RatingHistoryChartProps) {
  const { t } = useTranslation();
  const svgRef = useRef<SVGSVGElement>(null);
  const [wrapRef, width] = useMeasuredWidth();
  const [hover, setHover] = useState<ChartPoint | null>(null);
  const [drag, setDrag] = useState<{ start: number; end: number } | null>(null);

  const parsed = useMemo(() => points
    .map((point) => ({
      timestamp: new Date(point.timestamp).valueOf(),
      rating: Number(point.rating),
    }))
    .filter((point) => Number.isFinite(point.timestamp) && Number.isFinite(point.rating)), [points]);

  const domain = useMemo(() => {
    if (parsed.length === 0) return null;
    const times = extent(parsed.map((point) => point.timestamp));
    const ratings = extent(parsed.map((point) => point.rating));
    let { min: minRating, max: maxRating } = ratings;

    const maximumTime = maximum ? new Date(maximum.timestamp).valueOf() : Number.NaN;
    const maximumRating = Number(maximum?.rating);
    if (Number.isFinite(maximumTime) && Number.isFinite(maximumRating)
      && maximumTime >= times.min && maximumTime <= times.max) {
      minRating = Math.min(minRating, maximumRating);
      maxRating = Math.max(maxRating, maximumRating);
    }

    // The old padding floor was a flat 50 rating points, which for a player
    // whose whole history spans ~130 points left the line sitting in the
    // middle 45% of the box. Proportional padding with a small floor keeps a
    // flat history readable without wasting half the plot.
    const padding = Math.max((maxRating - minRating) * 0.15, 15);
    const step = niceStep(((maxRating + padding) - (minRating - padding)) / 4);
    return {
      minTime: times.min,
      maxTime: Math.max(times.max, times.min + 1),
      minRating: Math.floor((minRating - padding) / step) * step,
      maxRating: Math.ceil((maxRating + padding) / step) * step,
      step,
    };
  }, [maximum, parsed]);

  const project = useMemo(() => {
    if (!domain) return null;
    const plotWidth = Math.max(width - LEFT - RIGHT, 1);
    const plotHeight = HEIGHT - TOP - BOTTOM;
    return {
      x: (timestamp: number) =>
        LEFT + ((timestamp - domain.minTime) / (domain.maxTime - domain.minTime)) * plotWidth,
      y: (rating: number) =>
        TOP + (1 - (rating - domain.minRating) / (domain.maxRating - domain.minRating)) * plotHeight,
    };
  }, [domain, width]);

  const chart = useMemo<ChartPoint[]>(() => {
    if (!project) return [];
    return downsample(parsed).map((point) => ({
      ...point,
      x: project.x(point.timestamp),
      y: project.y(point.rating),
    }));
  }, [parsed, project]);

  const chartX = (event: ReactMouseEvent<SVGSVGElement>) => {
    const rect = svgRef.current?.getBoundingClientRect();
    return rect ? event.clientX - rect.left : LEFT;
  };
  const nearest = (x: number) => chart.reduce<ChartPoint | null>((best, point) => (
    best === null || Math.abs(point.x - x) < Math.abs(best.x - x) ? point : best
  ), null);

  const selection = useMemo(() => {
    if (!drag || !domain || !project || chart.length === 0) return null;
    const plotWidth = Math.max(width - LEFT - RIGHT, 1);
    const toTime = (x: number) =>
      domain.minTime + ((x - LEFT) / plotWidth) * (domain.maxTime - domain.minTime);
    const lowTime = toTime(Math.min(drag.start, drag.end));
    const highTime = toTime(Math.max(drag.start, drag.end));
    const chosen = parsed.filter((point) => point.timestamp >= lowTime && point.timestamp <= highTime);
    if (chosen.length === 0) return null;
    const ratings = extent(chosen.map((point) => point.rating));
    return {
      start: chosen[0].timestamp,
      end: chosen[chosen.length - 1].timestamp,
      min: ratings.min,
      max: ratings.max,
      change: chosen[chosen.length - 1].rating - chosen[0].rating,
      games: chosen.length,
    };
  }, [chart.length, domain, drag, parsed, project, width]);

  const maximumPoint = useMemo<ChartPoint | null>(() => {
    const source = maximum && Number.isFinite(Number(maximum.rating)) && !Number.isNaN(new Date(maximum.timestamp).valueOf())
      ? { timestamp: new Date(maximum.timestamp).valueOf(), rating: Number(maximum.rating) }
      : parsed.reduce<(typeof parsed)[number] | null>((best, point) => best === null || point.rating > best.rating ? point : best, null);
    if (!source || !project) return null;
    return { ...source, x: project.x(source.timestamp), y: project.y(source.rating) };
  }, [maximum, parsed, project]);

  if (!domain || !project || chart.length === 0) {
    return <div className="player-card-chart-empty surface-panel muted">{t("playerCard.chart.empty")}</div>;
  }

  const tickCount = 5;
  const yTicks: number[] = [];
  for (let tick = domain.minRating; tick <= domain.maxRating + 0.001; tick += domain.step) {
    yTicks.push(tick);
  }
  const xTicks = Array.from({ length: tickCount }, (_, index) =>
    domain.minTime + ((domain.maxTime - domain.minTime) * index) / (tickCount - 1));

  const line = chart.map((point) => `${point.x},${point.y}`).join(" ");
  // Closing the polyline to the baseline gives the series weight without
  // making the stroke itself louder than the player's own name.
  const area = `${LEFT},${HEIGHT - BOTTOM} ${line} ${chart[chart.length - 1].x},${HEIGHT - BOTTOM}`;
  // The peak label flips to the left of the marker in the right-hand third, so
  // it never runs off the plot.
  const peakAtEnd = maximumPoint !== null && maximumPoint.x > LEFT + (width - LEFT - RIGHT) * 0.66;

  return (
    <div className="player-rating-chart-wrap surface-panel" ref={wrapRef}>
      {selection && (
        <div className="player-rating-selection">
          <span className="player-rating-selection-range">
            {t("playerCard.chart.range", { start: formatDate(selection.start), end: formatDate(selection.end) })}
          </span>
          <span>{t("playerCard.chart.games", { count: formatNumber(selection.games) })}</span>
          <span>{t("playerCard.chart.low")} <strong>{selection.min.toFixed(0)}</strong></span>
          <span>{t("playerCard.chart.high")} <strong>{selection.max.toFixed(0)}</strong></span>
          <span className={selection.change >= 0 ? "is-gain" : "is-loss"}>
            {selection.change >= 0 ? "+" : ""}{selection.change.toFixed(0)}
          </span>
          <button type="button" className="player-rating-selection-clear" onClick={() => setDrag(null)}>
            {t("playerCard.chart.clear")}
          </button>
        </div>
      )}
      <svg
        ref={svgRef}
        className="player-rating-chart"
        width={width}
        height={HEIGHT}
        viewBox={`0 0 ${width} ${HEIGHT}`}
        role="img"
        aria-label={t("playerCard.chart.aria", { count: parsed.length })}
        onMouseDown={(event) => setDrag({ start: chartX(event), end: chartX(event) })}
        onMouseMove={(event) => {
          const x = chartX(event);
          if (event.buttons === 1) setDrag((current) => current ? { ...current, end: x } : current);
          setHover(nearest(x));
        }}
        onMouseLeave={() => setHover(null)}
      >
        {yTicks.map((tick) => (
          <g key={tick}>
            <line className="chart-grid" x1={LEFT} x2={width - RIGHT} y1={project.y(tick)} y2={project.y(tick)} />
            <text className="chart-label" x={LEFT - 10} y={project.y(tick) + 4}>{tick.toFixed(0)}</text>
          </g>
        ))}
        {/* Only the baseline and the two edges are drawn vertically: a full
            grid competed with the series it was meant to support. */}
        <line className="chart-axis" x1={LEFT} x2={width - RIGHT} y1={HEIGHT - BOTTOM} y2={HEIGHT - BOTTOM} />
        {xTicks.map((tick, index) => {
          const x = LEFT + (index / (tickCount - 1)) * (width - LEFT - RIGHT);
          const anchor = index === 0 ? "chart-date-start" : index === tickCount - 1 ? "chart-date-end" : "chart-date-middle";
          return <text key={tick} className={`chart-label ${anchor}`} x={x} y={HEIGHT - 10}>{formatDate(tick)}</text>;
        })}

        <polygon className="chart-rating-area" points={area} />
        <polyline className="chart-rating-line" points={line} />

        {showMaximum && maximumPoint && maximumPoint.timestamp >= domain.minTime && maximumPoint.timestamp <= domain.maxTime && (
          <g className="chart-peak">
            <line className="chart-max-line" x1={LEFT} x2={width - RIGHT} y1={maximumPoint.y} y2={maximumPoint.y} />
            <circle className="chart-peak-dot" cx={maximumPoint.x} cy={maximumPoint.y} r={3.5} />
            <text
              className={`chart-max-label ${peakAtEnd ? "chart-date-end" : "chart-date-start"}`}
              x={peakAtEnd ? maximumPoint.x - 9 : maximumPoint.x + 9}
              y={maximumPoint.y - 8}
            >
              {t("playerCard.chart.peak", { rating: maximumPoint.rating.toFixed(0) })}
            </text>
          </g>
        )}

        {drag && Math.abs(drag.end - drag.start) > 1 && (
          <rect
            className="chart-selection"
            x={Math.min(drag.start, drag.end)}
            y={TOP}
            width={Math.abs(drag.end - drag.start)}
            height={HEIGHT - TOP - BOTTOM}
          />
        )}
        {hover && (
          <g>
            <line className="chart-crosshair" x1={hover.x} x2={hover.x} y1={TOP} y2={HEIGHT - BOTTOM} />
            <circle className="chart-point" cx={hover.x} cy={hover.y} r={4} />
          </g>
        )}
      </svg>
      {hover && (
        <div
          className={`player-rating-tooltip surface-raised${hover.x > width / 2 ? " is-left" : ""}`}
          style={{ left: hover.x, top: hover.y }}
        >
          <strong>{hover.rating.toFixed(0)}</strong>
          <span>{formatDateTime(hover.timestamp)}</span>
        </div>
      )}
    </div>
  );
}
