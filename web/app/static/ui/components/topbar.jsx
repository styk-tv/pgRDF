// Top header + mode sub-bar for pgRDF Console.

const { useState } = React;

function BrandMark() {
  return (
    <svg viewBox="0 0 20 20" fill="none">
      <circle cx="5" cy="6"  r="2.4" stroke="#0e0e0e" strokeWidth="1.1" fill="#fff"/>
      <circle cx="15" cy="6" r="2.4" fill="#0e0e0e"/>
      <circle cx="10" cy="14" r="2.4" stroke="#0e0e0e" strokeWidth="1.1" fill="#fff"/>
      <path d="M7 7.5 L13 12.5 M13 7.5 L7 12.5" stroke="#0e0e0e" strokeWidth="0.9"/>
    </svg>
  );
}

function TopBar({ onRun, onExplain, onSave, running, conn }) {
  const c = conn || {};
  return (
    <header className="head">
      <div className="brand">
        <span className="brand-mark"><BrandMark/></span>
        <span className="pg">pg</span><span className="rdf">RDF</span>
        <span style={{ color: "var(--ink-3)", fontWeight: 400, marginLeft: 2 }}>Console</span>
      </div>
      <span className="brand-badge">{c.version ? `pgrdf ${c.version}` : "pgrdf"}</span>
      <span className="brand-badge" style={{ color: "var(--accent)", borderColor: "var(--accent-line)", background: "var(--accent-soft)" }}>
        {c.live ? "live" : "fixtures"}
      </span>

      <div className="head-sep"/>

      <div className="conn" title={c.live ? "Live connection to project database" : "Not connected — showing fixtures"}>
        <span className="conn-dot" style={c.live ? { background: "var(--ok, #2e9e5b)" } : { background: "var(--ink-3)" }}/>
        <span className="conn-host">project@</span>
        <span className="conn-db">{c.project || "—"}</span>
        <span className="conn-meta">· {c.db || "—"}</span>
      </div>

      <div className="head-grow"/>

      <button className="head-btn" onClick={onSave}>
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
          <path d="M2 2 H8 L10 4 V10 H2 Z M4 2 V5 H8 V2 M4 10 V7 H8 V10" stroke="currentColor" strokeWidth="0.9"/>
        </svg>
        Save
        <span className="k">⌘S</span>
      </button>
      <button className="head-btn" title="History">
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
          <circle cx="6" cy="6" r="4.2" stroke="currentColor" strokeWidth="0.9"/>
          <path d="M6 3.5 V6 L7.6 7.4" stroke="currentColor" strokeWidth="0.9" strokeLinecap="round"/>
        </svg>
        History
      </button>
      <button className="head-btn" onClick={onExplain}>
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none">
          <path d="M2 9 L5 4 L7 7 L10 2" stroke="currentColor" strokeWidth="0.9" fill="none"/>
        </svg>
        Explain
      </button>
      <button className="head-btn primary" onClick={onRun} disabled={running}>
        <svg width="10" height="10" viewBox="0 0 10 10">
          <path d="M2 1 L9 5 L2 9 Z" fill="currentColor"/>
        </svg>
        {running ? "Running…" : "Run"}
        <span className="k">⌘↵</span>
      </button>
    </header>
  );
}

function SubBar({ mode, setMode, view, dbStats }) {
  return (
    <div className="sub">
      <div className="mode">
        <button className={mode === "visual" ? "active" : ""} onClick={() => setMode("visual")}>
          ◇ VISUAL
        </button>
        <button className={mode === "sparql" ? "active" : ""} onClick={() => setMode("sparql")}>
          ⌘ SPARQL
        </button>
        <button className={mode === "sql" ? "active" : ""} onClick={() => setMode("sql")}>
          ⌥ SQL
        </button>
      </div>

      <div className="crumbs">
        <span>research_db</span>
        <span className="sep">/</span>
        <span>lubm:2024</span>
        <span className="sep">/</span>
        <span className="here">{view}</span>
      </div>

      <div className="sub-meta">
        <span>graph <b>lubm:2024</b></span>
        <span>triples <b>{dbStats.triples}</b></span>
        <span>inferred <b>{dbStats.inferred}</b></span>
        <span>dict <b>{dbStats.dict}</b></span>
        <span>shmem cache <b style={{ color: "var(--ok)" }}>{dbStats.cache}</b></span>
      </div>
    </div>
  );
}

window.TopBar = TopBar;
window.SubBar = SubBar;
