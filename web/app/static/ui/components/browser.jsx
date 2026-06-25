// Schema browser views — Classes / Predicates / Shapes / Rules / Graphs.
// These render when the user clicks a sidebar item; they live in the
// main pane where the canvas usually sits.

function StatusDot({ kind }) {
  const c = kind === "materialized" ? "var(--ok)" :
            kind === "stale" ? "var(--warn)" : "var(--ink-4)";
  return <span style={{ display: "inline-block", width: 6, height: 6, borderRadius: 3, background: c, marginRight: 6 }}/>;
}

function BrowserHead({ title, summary, children }) {
  return (
    <div className="browser-head">
      <h1>{title}</h1>
      <span className="summary">{summary}</span>
      <div style={{ marginLeft: "auto", display: "flex", gap: 6 }}>{children}</div>
    </div>
  );
}

function PredicatesView({ active }) {
  const D = window.PGRDF_DATA;
  const maxU = Math.max(...D.PREDICATES.map(p => p.uses));
  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "#fff" }}>
      <BrowserHead
        title="Predicates"
        summary={`${D.PREDICATES.length} predicates · ${D.PREDICATES.reduce((s, p) => s + p.uses, 0).toLocaleString()} total uses across all named graphs`}>
        <button className="head-btn">Use in query</button>
        <button className="head-btn">Export</button>
      </BrowserHead>
      <div className="browser" style={{ flex: 1 }}>
        <table>
          <thead>
            <tr>
              <th style={{ width: "36%" }}>predicate</th>
              <th style={{ width: 80 }}>group</th>
              <th>domain</th>
              <th>range</th>
              <th style={{ width: 160 }}>uses</th>
            </tr>
          </thead>
          <tbody>
            {D.PREDICATES.map(p => (
              <tr key={p.iri} className={active?.pred === p.iri ? "active" : ""}>
                <td><span className="iri">{p.iri}</span></td>
                <td><span className={"chip " + (p.group === "ub" ? "pred" : "muted")}>{p.group}</span></td>
                <td>{p.domain ? <span className="chip class">{p.domain}</span> : <span style={{ color: "var(--ink-4)" }}>—</span>}</td>
                <td>{p.range ? <span className="chip class">{p.range}</span> : <span style={{ color: "var(--ink-4)" }}>—</span>}</td>
                <td className="uses">
                  {p.uses.toLocaleString()}
                  <span className="barwrap"><span className="bar" style={{ width: (p.uses / maxU * 100) + "%" }}/></span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function ClassesView({ active }) {
  const D = window.PGRDF_DATA;
  const maxI = Math.max(...D.CLASSES.map(c => c.instances));
  // Build a quick subclass tree (one-pass; not deep)
  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "#fff" }}>
      <BrowserHead
        title="Classes"
        summary={`${D.CLASSES.length} classes registered · OWL2-RL profile · ${D.CLASSES.reduce((s,c)=>s+c.instances,0).toLocaleString()} total instances`}>
        <button className="head-btn">Use in query</button>
        <button className="head-btn">Materialize subclasses</button>
      </BrowserHead>
      <div className="browser" style={{ flex: 1 }}>
        <table>
          <thead>
            <tr>
              <th style={{ width: "36%" }}>class</th>
              <th>subClassOf</th>
              <th style={{ width: 80 }}>profile</th>
              <th style={{ width: 200 }}>instances</th>
            </tr>
          </thead>
          <tbody>
            {D.CLASSES.map(c => (
              <tr key={c.iri} className={active?.cls === c.iri ? "active" : ""}>
                <td>
                  <span className="chip class">{c.iri}</span>
                </td>
                <td>{c.subOf ? <span className="iri">{c.subOf}</span> : <span style={{ color: "var(--ink-4)" }}>—</span>}</td>
                <td><span className="chip muted">OWL2-RL</span></td>
                <td className="uses">
                  {c.instances.toLocaleString()}
                  <span className="barwrap"><span className="bar" style={{ width: (c.instances / maxI * 100) + "%" }}/></span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function ShapesView({ active }) {
  const D = window.PGRDF_DATA;
  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "#fff" }}>
      <BrowserHead title="SHACL Shapes"
        summary={`${D.SHAPES.length} shape nodes · last report 12:31:08 · 4 violations, 2 warnings`}>
        <button className="head-btn">Re-validate</button>
        <button className="head-btn">Report (JSONB)</button>
      </BrowserHead>
      <div className="browser" style={{ flex: 1 }}>
        <table>
          <thead>
            <tr>
              <th>shape</th>
              <th>targets</th>
              <th style={{ width: 100 }}>constraints</th>
              <th style={{ width: 120 }}>severity</th>
              <th>last violation</th>
            </tr>
          </thead>
          <tbody>
            {D.SHAPES.map(s => (
              <tr key={s.iri} className={active?.shape === s.iri ? "active" : ""}>
                <td><span className="chip shape">{s.iri}</span></td>
                <td><span className="iri">{s.targets}</span></td>
                <td className="uses">{s.constraints}</td>
                <td>
                  <span className="chip" style={{
                    color: s.severity === "Violation" ? "var(--err)" : "var(--warn)",
                    background: s.severity === "Violation" ? "#fbe6e6" : "#fbeede",
                    borderColor: s.severity === "Violation" ? "#f0c3c3" : "#f0d6a8",
                  }}>{s.severity}</span>
                </td>
                <td style={{ color: "var(--ink-3)" }}>2 min ago</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function RulesView({ active }) {
  const D = window.PGRDF_DATA;
  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "#fff" }}>
      <BrowserHead title="Inference Rules"
        summary="reasonable 0.4 · OWL2-RL profile · 312,445 inferred triples in lubm:inferred">
        <button className="head-btn">Materialize all</button>
        <button className="head-btn">Drop inferred</button>
      </BrowserHead>
      <div className="browser" style={{ flex: 1 }}>
        <table>
          <thead>
            <tr>
              <th>rule</th>
              <th style={{ width: 100 }}>kind</th>
              <th style={{ width: 120 }}>status</th>
              <th style={{ width: 200 }}>derives</th>
              <th>last run</th>
            </tr>
          </thead>
          <tbody>
            {D.RULES.map(r => (
              <tr key={r.iri} className={active?.rule === r.iri ? "active" : ""}>
                <td><span className="iri">{r.iri}</span></td>
                <td><span className="chip muted">{r.kind}</span></td>
                <td><StatusDot kind={r.status}/>{r.status}</td>
                <td className="uses">{r.derives.toLocaleString()}</td>
                <td style={{ color: "var(--ink-3)" }}>
                  {r.status === "stale" ? "4h ago — stale" : r.status === "disabled" ? "—" : "12:14"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function GraphsView({ active }) {
  const D = window.PGRDF_DATA;
  const total = D.GRAPHS.reduce((s, g) => s + g.triples, 0);
  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "#fff" }}>
      <BrowserHead title="Named Graphs"
        summary={`${D.GRAPHS.length} graphs · ${total.toLocaleString()} total triples · partition-by-list on graph_id`}>
        <button className="head-btn">Import…</button>
        <button className="head-btn">+ New graph</button>
      </BrowserHead>
      <div className="browser" style={{ flex: 1 }}>
        <table>
          <thead>
            <tr>
              <th>graph</th>
              <th style={{ width: 140 }}>partition</th>
              <th style={{ width: 100 }}>kind</th>
              <th style={{ width: 200 }}>triples</th>
              <th>updated</th>
            </tr>
          </thead>
          <tbody>
            {D.GRAPHS.map(g => (
              <tr key={g.id} className={active?.graph === g.id ? "active" : ""}>
                <td><span className="chip graph">{g.label}</span></td>
                <td><span className="iri">_pgrdf_quads_g{g.id}</span></td>
                <td><span className="chip muted">{g.inferred ? "inferred" : "asserted"}</span></td>
                <td className="uses">{g.triples.toLocaleString()}</td>
                <td style={{ color: "var(--ink-3)" }}>{g.inferred ? "12:14 (re-mat)" : "2026-05-13"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

Object.assign(window, { PredicatesView, ClassesView, ShapesView, RulesView, GraphsView });
