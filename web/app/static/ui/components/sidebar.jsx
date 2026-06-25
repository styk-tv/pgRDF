// Left-side schema / asset browser. Sections collapse; items can drive
// the active view in the main pane.

const { useState: useStateSide } = React;

function SideSection({ id, title, count, defaultOpen = true, children, open, setOpen }) {
  const isOpen = open[id] ?? defaultOpen;
  return (
    <div className="side-sect">
      <div className={"side-sect-h " + (isOpen ? "open" : "")}
           onClick={() => setOpen({ ...open, [id]: !isOpen })}>
        <span className="chev">›</span>
        <span>{title}</span>
        {count != null && <span className="count">{count}</span>}
      </div>
      {isOpen && <div>{children}</div>}
    </div>
  );
}

function SideItem({ pill, pillKind = "sq", label, n, active, onClick, title }) {
  const pillCls = "pill " + (pillKind === "circ" ? "circ" : "");
  return (
    <div className={"side-item " + (active ? "active" : "")} onClick={onClick} title={title}>
      {pill && <span className={pillCls} style={{ background: pill }}/>}
      <span className="lbl">{label}</span>
      {n != null && <span className="n">{n}</span>}
    </div>
  );
}

const COLORS = {
  graph: "#2f6fb0",
  class: "#5b3aa6",
  pred:  "#1f6f7a",
  shape: "#a8541f",
  rule:  "#0e0e0e",
  saved: "#5a5a5a",
};

const fmt = (n) => n.toLocaleString();

function Sidebar({ view, setView, active, setActive }) {
  const D = window.PGRDF_DATA;
  const [open, setOpen] = useStateSide({
    graphs: true, classes: true, predicates: true,
    shapes: false, rules: false, saved: true,
  });
  const [filter, setFilter] = useStateSide("");
  const ff = filter.toLowerCase();
  const m = (s) => !ff || s.toLowerCase().includes(ff);

  return (
    <aside className="side">
      <div className="side-search">
        <input
          placeholder="Filter schema…  ⌘K"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>

      <SideSection id="graphs" title="Named graphs" count={D.GRAPHS.length} open={open} setOpen={setOpen}>
        {D.GRAPHS.filter(g => m(g.label)).map(g => (
          <SideItem
            key={g.id}
            pill={g.inferred ? "transparent" : COLORS.graph}
            pillKind="circ"
            label={g.label}
            n={fmt(g.triples)}
            active={view === "graph" && active.graph === g.id}
            onClick={() => { setView("graph"); setActive({ ...active, graph: g.id }); }}
            title={g.inferred ? "Inferred graph" : "Asserted graph"}
          />
        ))}
      </SideSection>

      <SideSection id="classes" title="Classes" count={D.CLASSES.length} open={open} setOpen={setOpen}>
        {D.CLASSES.filter(c => m(c.iri)).map(c => (
          <SideItem
            key={c.iri}
            pill={COLORS[c.color] || COLORS.class}
            label={c.iri}
            n={fmt(c.instances)}
            active={view === "classes" && active.cls === c.iri}
            onClick={() => { setView("classes"); setActive({ ...active, cls: c.iri }); }}
          />
        ))}
      </SideSection>

      <SideSection id="predicates" title="Predicates" count={D.PREDICATES.length} open={open} setOpen={setOpen}>
        {D.PREDICATES.filter(p => m(p.iri)).map(p => (
          <SideItem
            key={p.iri}
            pill={COLORS.pred}
            label={p.iri}
            n={fmt(p.uses)}
            active={view === "predicates" && active.pred === p.iri}
            onClick={() => { setView("predicates"); setActive({ ...active, pred: p.iri }); }}
          />
        ))}
      </SideSection>

      <SideSection id="shapes" title="SHACL shapes" count={D.SHAPES.length} defaultOpen={false} open={open} setOpen={setOpen}>
        {D.SHAPES.filter(s => m(s.iri)).map(s => (
          <SideItem
            key={s.iri}
            pill={COLORS.shape}
            label={s.iri}
            n={s.constraints + "c"}
            active={view === "shapes" && active.shape === s.iri}
            onClick={() => { setView("shapes"); setActive({ ...active, shape: s.iri }); }}
          />
        ))}
      </SideSection>

      <SideSection id="rules" title="Inference rules" count={D.RULES.length} defaultOpen={false} open={open} setOpen={setOpen}>
        {D.RULES.filter(r => m(r.iri)).map(r => (
          <SideItem
            key={r.iri}
            pill={r.status === "materialized" ? COLORS.rule : (r.status === "stale" ? "#b8741f" : "#b8b8b8")}
            pillKind="circ"
            label={r.iri}
            n={fmt(r.derives)}
            active={view === "rules" && active.rule === r.iri}
            onClick={() => { setView("rules"); setActive({ ...active, rule: r.iri }); }}
            title={r.status}
          />
        ))}
      </SideSection>

      <SideSection id="saved" title="Saved queries" count={D.SAVED.length} open={open} setOpen={setOpen}>
        {D.SAVED.filter(q => m(q.name)).map(q => (
          <SideItem
            key={q.id}
            pill={COLORS.saved}
            label={q.name}
            n={q.updated}
            active={view === "query" && active.saved === q.id}
            onClick={() => { setView("query"); setActive({ ...active, saved: q.id }); }}
          />
        ))}
      </SideSection>

      <div style={{ padding: "10px 14px 16px", color: "var(--ink-3)", fontFamily: "var(--mono)", fontSize: 10, lineHeight: 1.6 }}>
        <div style={{ display: "flex", justifyContent: "space-between" }}>
          <span>extension</span><span style={{ color: "var(--ink-1)" }}>pgrdf 0.2.1</span>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between" }}>
          <span>postgres</span><span style={{ color: "var(--ink-1)" }}>17.2 · arm64</span>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between" }}>
          <span>reasoner</span><span style={{ color: "var(--ink-1)" }}>reasonable 0.4</span>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between" }}>
          <span>shacl</span><span style={{ color: "var(--ink-1)" }}>shacl-rust 0.1</span>
        </div>
      </div>
    </aside>
  );
}

window.Sidebar = Sidebar;
