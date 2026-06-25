// Visual SPARQL query designer.
// Nodes are draggable on a 24px grid; edges are orthogonal 3-segment polylines
// that exit nodes through the side closest to the target.

const { useState: useStateC, useRef: useRefC, useEffect: useEffectC, useMemo } = React;

const NODE_R = 44;        // node circle radius
const GRID = 24;
const SNAP = (v) => Math.round(v / GRID) * GRID;

// Compute orthogonal route between two nodes (a -> b) and the label anchor.
// Returns { points: [[x,y],...], label: [x,y], arrowAt: [x,y,angle] }.
function route(a, b) {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const horiz = Math.abs(dx) >= Math.abs(dy);
  const sx = a.x + (horiz ? (dx > 0 ? NODE_R : -NODE_R) : 0);
  const sy = a.y + (horiz ? 0 : (dy > 0 ? NODE_R : -NODE_R));
  const tx = b.x + (horiz ? (dx > 0 ? -NODE_R : NODE_R) : 0);
  const ty = b.y + (horiz ? 0 : (dy > 0 ? -NODE_R : NODE_R));

  // 3-segment polyline: midpoint along the dominant axis.
  let pts, labelXY, arrow;
  if (horiz) {
    const mx = (sx + tx) / 2;
    pts = [[sx, sy], [mx, sy], [mx, ty], [tx, ty]];
    labelXY = [mx, (sy + ty) / 2];
    arrow = [tx, ty, dx > 0 ? 0 : 180];
  } else {
    const my = (sy + ty) / 2;
    pts = [[sx, sy], [sx, my], [tx, my], [tx, ty]];
    labelXY = [(sx + tx) / 2, my];
    arrow = [tx, ty, dy > 0 ? 90 : -90];
  }
  return { points: pts, labelXY, arrow };
}

function toPath(points) {
  return points.map((p, i) => (i ? "L" : "M") + p[0] + " " + p[1]).join(" ");
}

function Node({ n, selected, onSelect, onDrag, nodeStyle, scale }) {
  const ref = useRefC(null);
  const drag = useRefC(null);

  const onDown = (e) => {
    e.preventDefault();
    onSelect(n.id);
    drag.current = { ox: e.clientX, oy: e.clientY, nx: n.x, ny: n.y };
    const move = (ev) => {
      const d = drag.current;
      if (!d) return;
      const s = scale || 1;
      const nx = SNAP(d.nx + (ev.clientX - d.ox) / s);
      const ny = SNAP(d.ny + (ev.clientY - d.oy) / s);
      onDrag(n.id, nx, ny);
    };
    const up = () => {
      drag.current = null;
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // Position is the node CENTER; offset by NODE_R for the wrapping div.
  const left = n.x - NODE_R;
  const top  = n.y - NODE_R;

  const isLit = n.kind === "lit";
  const cls = "node " + n.kind + (selected ? " selected" : "");

  return (
    <div ref={ref} className={cls}
         style={{ left, top, width: NODE_R * 2 }}
         onMouseDown={onDown}>
      <div className="node-circle" style={{
        borderRadius: nodeStyle === "pill" ? 999 : (nodeStyle === "box" ? 6 : "50%"),
        width: nodeStyle === "pill" ? 110 : NODE_R * 2,
        height: nodeStyle === "pill" ? 44 : NODE_R * 2,
        marginTop: nodeStyle === "pill" ? 22 : 0,
      }}>
        <span className="node-label">{n.label}</span>
        {n.projected && <span className="node-projected">{n.projected}</span>}
      </div>
      {n.type && (
        <span className={"node-sub" + (isLit ? " lit-tag" : "")}>
          {isLit ? "↦ " : "a "} {n.type}
        </span>
      )}
    </div>
  );
}

function Canvas({ nodes, edges, onMove, selected, setSelected, nodeStyle = "circle" }) {
  const wrapRef = useRefC(null);
  const [size, setSize] = useStateC({ w: 800, h: 500 });

  useEffectC(() => {
    if (!wrapRef.current) return;
    const ro = new ResizeObserver(() => {
      const r = wrapRef.current.getBoundingClientRect();
      setSize({ w: r.width, h: r.height });
    });
    ro.observe(wrapRef.current);
    return () => ro.disconnect();
  }, []);

  const byId = useMemo(() => Object.fromEntries(nodes.map(n => [n.id, n])), [nodes]);

  // Pre-route all edges
  const routed = edges.map(e => ({
    e,
    r: route(byId[e.from], byId[e.to]),
  }));

  // Auto-fit: compute bbox of nodes, scale + translate so content centers in canvas.
  const pad = 70;
  const xs = nodes.map(n => n.x), ys = nodes.map(n => n.y);
  const minX = Math.min(...xs) - pad, maxX = Math.max(...xs) + pad;
  const minY = Math.min(...ys) - pad - 30, maxY = Math.max(...ys) + pad + 30; // extra room for type chips
  const bbW = maxX - minX, bbH = maxY - minY;
  const scale = Math.min(size.w / bbW, size.h / bbH, 1.15);
  const tx = (size.w - bbW * scale) / 2 - minX * scale;
  const ty = (size.h - bbH * scale) / 2 - minY * scale;
  const transform = `translate(${tx}px, ${ty}px) scale(${scale})`;

  return (
    <div className="canvas-wrap" ref={wrapRef} onMouseDown={(ev) => {
      if (ev.target === ev.currentTarget) setSelected(null);
    }}>
      <CanvasToolbar />
      <CanvasLegend />

      <div className="canvas-content" style={{
        position: "absolute", inset: 0, transformOrigin: "0 0", transform,
        pointerEvents: "none",
      }}>
      <svg className="canvas-svg" viewBox={`0 0 ${size.w} ${size.h}`}
           style={{ overflow: "visible", pointerEvents: "none" }}>
        <defs>
          <marker id="arrow" viewBox="0 0 8 8" refX="6.5" refY="4"
                  markerWidth="6" markerHeight="6" orient="auto-start-reverse">
            <path d="M0 0 L8 4 L0 8 Z" className="edge-arrow"/>
          </marker>
        </defs>

        {routed.map(({ e, r }) => {
          const [lx, ly] = r.labelXY;
          const w = e.pred.length * 6.6 + 16;
          return (
            <g key={e.id}>
              <path d={toPath(r.points)}
                    className={"edge-line" + (e.optional ? " optional" : "")}
                    markerEnd="url(#arrow)"/>
              <rect x={lx - w / 2} y={ly - 9} width={w} height={18} rx={2}
                    className="edge-label-bg"/>
              <text x={lx} y={ly + 3.5} textAnchor="middle" className="edge-label">
                {e.pred}
              </text>
              {e.optional && (
                <text x={lx - w/2 - 4} y={ly + 3} textAnchor="end"
                      style={{ fontFamily: "var(--mono)", fontSize: 9, fill: "var(--ink-3)" }}>
                  OPT
                </text>
              )}
            </g>
          );
        })}
      </svg>

      <div style={{ pointerEvents: "auto" }}>
        {nodes.map(n => (
          <Node key={n.id} n={n}
                selected={selected === n.id}
                onSelect={setSelected}
                onDrag={onMove}
                nodeStyle={nodeStyle}
                scale={scale} />
        ))}
      </div>
      </div>

      <CanvasMinimap nodes={nodes} routed={routed} size={size}/>
    </div>
  );
}

function CanvasToolbar() {
  return (
    <div className="canvas-toolbar">
      <div className="group">
        <button title="Add ?variable">+ ?var</button>
        <button title="Add a constant IRI">+ IRI</button>
        <button title="Add a literal">+ literal</button>
        <button title="Add a triple pattern">+ triple</button>
      </div>
      <div className="group">
        <button title="Add FILTER clause">FILTER</button>
        <button title="Wrap in OPTIONAL">OPTIONAL</button>
        <button title="Wrap in UNION">UNION</button>
      </div>
      <div className="group">
        <button className="icon" title="Auto layout">⇄</button>
        <button className="icon" title="Fit">⤢</button>
        <button className="icon" title="Zoom in">＋</button>
        <button className="icon" title="Zoom out">－</button>
      </div>
    </div>
  );
}

function CanvasLegend() {
  return (
    <div className="canvas-legend">
      <div className="row">
        <svg width="14" height="14"><circle cx="7" cy="7" r="5" fill="#fff" stroke="#5a5a5a" strokeWidth="1" strokeDasharray="3 2"/></svg>
        <span>?variable</span>
      </div>
      <div className="row">
        <svg width="14" height="14"><circle cx="7" cy="7" r="5" fill="#f4f4f3" stroke="#0e0e0e" strokeWidth="1"/></svg>
        <span>IRI / class</span>
      </div>
      <div className="row">
        <svg width="14" height="14"><rect x="2" y="3" width="10" height="8" rx="1" fill="#f4f4f3" stroke="#0e0e0e" strokeWidth="1"/></svg>
        <span>literal</span>
      </div>
      <div className="row">
        <svg width="22" height="6"><line x1="0" y1="3" x2="22" y2="3" stroke="#5a5a5a" strokeDasharray="3 2"/></svg>
        <span>OPTIONAL</span>
      </div>
    </div>
  );
}

function CanvasMinimap({ nodes, size }) {
  if (!nodes.length) return null;
  const padded = 60;
  const xs = nodes.map(n => n.x), ys = nodes.map(n => n.y);
  const minX = Math.min(...xs) - padded, maxX = Math.max(...xs) + padded;
  const minY = Math.min(...ys) - padded, maxY = Math.max(...ys) + padded;
  const w = maxX - minX, h = maxY - minY;

  return (
    <div className="canvas-minimap">
      <div className="canvas-minimap-label">MAP</div>
      <svg viewBox={`${minX} ${minY} ${w} ${h}`} width="100%" height="100%" preserveAspectRatio="xMidYMid meet">
        {nodes.map(n => (
          <circle key={n.id} cx={n.x} cy={n.y} r="14"
                  fill={n.kind === "var" ? "#fff" : "#dadada"}
                  stroke="#0e0e0e" strokeWidth="1.5"
                  strokeDasharray={n.kind === "var" ? "5 3" : ""}/>
        ))}
      </svg>
    </div>
  );
}

window.Canvas = Canvas;
