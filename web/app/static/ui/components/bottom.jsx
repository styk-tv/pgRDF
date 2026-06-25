// Bottom panel: SPARQL editor / generated SQL / results / plan / log.

const { useState: useStateB, useRef: useRefB, useEffect: useEffectB } = React;

// ── Syntax highlight (tiny, hand-rolled — not a full tokenizer) ─────────
function highlightSparql(src) {
  const KW = /\b(PREFIX|SELECT|DISTINCT|WHERE|FILTER|OPTIONAL|UNION|GRAPH|ORDER|BY|LIMIT|OFFSET|GROUP|HAVING|ASK|CONSTRUCT|DESCRIBE|FROM|NAMED|BIND|AS|COUNT|SUM|AVG|MIN|MAX|a)\b/g;
  // Order: comments → strings → IRIs → vars → keywords → punctuation
  const lines = src.split("\n");
  return lines.map((line, i) => {
    let parts = [{ t: "_", v: line }];
    const apply = (re, cls) => {
      const out = [];
      for (const p of parts) {
        if (p.t !== "_") { out.push(p); continue; }
        let last = 0, m;
        re.lastIndex = 0;
        while ((m = re.exec(p.v))) {
          if (m.index > last) out.push({ t: "_", v: p.v.slice(last, m.index) });
          out.push({ t: cls, v: m[0] });
          last = m.index + m[0].length;
        }
        if (last < p.v.length) out.push({ t: "_", v: p.v.slice(last) });
      }
      parts = out;
    };
    apply(/#.*$/g, "com");
    apply(/"[^"]*"(?:@[A-Za-z-]+|\^\^\S+)?/g, "lit");
    apply(/<[^>]+>/g, "iri");
    apply(/\b[a-z][\w-]*:[\w-]*/g, "iri");  // prefixed names
    apply(/\?[A-Za-z_][\w]*/g, "var");
    apply(KW, "kw");
    apply(/[{}().,;]/g, "pun");
    return (
      <span key={i} className="ln">
        {parts.map((p, j) =>
          p.t === "_"
            ? p.v
            : <span key={j} className={"tok-" + p.t}>{p.v}</span>
        )}
        {"\n"}
      </span>
    );
  });
}

// Read-only highlighted view (used for the generated-SQL tab).
function Editor({ value, lang = "sparql" }) {
  const lines = value.split("\n");
  return (
    <div className="editor">
      <div className="editor-gutter">
        {lines.map((_, i) => <span key={i} className="ln">{i + 1}</span>)}
      </div>
      <div className="editor-code">
        {lang === "sparql" ? highlightSparql(value) : highlightSql(value)}
      </div>
    </div>
  );
}

// Editable SPARQL editor — a plain mono textarea with a line gutter.
// (Syntax-highlight overlay is a later cosmetic pass; functional first.)
function EditableEditor({ value, onChange }) {
  const lines = value.split("\n");
  return (
    <div className="editor editor-editable">
      <div className="editor-gutter">
        {lines.map((_, i) => <span key={i} className="ln">{i + 1}</span>)}
      </div>
      <textarea
        className="editor-textarea"
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        style={{
          flex: 1, border: "none", outline: "none", resize: "none",
          background: "transparent", color: "var(--ink)",
          font: '13px/1.55 var(--mono, ui-monospace, monospace)',
          padding: "2px 10px", whiteSpace: "pre", overflowWrap: "normal",
          overflowX: "auto",
        }}
      />
    </div>
  );
}

function highlightSql(src) {
  const KW = /\b(SELECT|FROM|WHERE|JOIN|INNER|LEFT|RIGHT|ON|AND|OR|NOT|IN|EXISTS|AS|GROUP|BY|ORDER|LIMIT|WITH|UNION|ALL)\b/g;
  const lines = src.split("\n");
  return lines.map((line, i) => {
    let parts = [{ t: "_", v: line }];
    const apply = (re, cls) => {
      const out = [];
      for (const p of parts) {
        if (p.t !== "_") { out.push(p); continue; }
        let last = 0, m;
        re.lastIndex = 0;
        while ((m = re.exec(p.v))) {
          if (m.index > last) out.push({ t: "_", v: p.v.slice(last, m.index) });
          out.push({ t: cls, v: m[0] });
          last = m.index + m[0].length;
        }
        if (last < p.v.length) out.push({ t: "_", v: p.v.slice(last) });
      }
      parts = out;
    };
    apply(/--.*$/g, "com");
    apply(/'[^']*'/g, "lit");
    apply(/\$\d+/g, "var");
    apply(KW, "kw");
    apply(/[(),.;]/g, "pun");
    return (
      <span key={i} className="ln">
        {parts.map((p, j) =>
          p.t === "_" ? p.v : <span key={j} className={"tok-" + p.t}>{p.v}</span>
        )}
        {"\n"}
      </span>
    );
  });
}

// ── Results table ──────────────────────────────────────────────────────
function Results({ cols, rows }) {
  return (
    <div className="results">
      <table>
        <thead>
          <tr>
            <th style={{ width: 38 }}>#</th>
            {cols.map(c => (
              <th key={c.v}>
                <span className="var">{c.label}</span>
                <span className="type">{c.type === "iri" ? "iri" : "literal"}</span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>
              <td className="row-num">{i + 1}</td>
              {r.map((v, j) => {
                const c = cols[j];
                return <td key={j} className={c.type === "iri" ? "iri" : "lit"}>{v}</td>;
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ── JSON-LD pretty raw ─────────────────────────────────────────────────
function JsonLd({ cols, rows }) {
  // Build a tiny SPARQL JSON results doc.
  const json = {
    head: { vars: cols.map(c => c.v) },
    results: {
      bindings: rows.map(row => {
        const o = {};
        row.forEach((v, i) => {
          const c = cols[i];
          o[c.v] = c.type === "iri"
            ? { type: "uri", value: v }
            : { type: "literal", value: v };
        });
        return o;
      }),
    },
  };
  // Custom pretty-printer with class spans
  const indent = (n) => "  ".repeat(n);
  const out = [];
  out.push("{\n");
  out.push(`${indent(1)}<span class="k">"head"</span>: {\n`);
  out.push(`${indent(2)}<span class="k">"vars"</span>: [`);
  out.push(json.head.vars.map(v => `<span class="s">"${v}"</span>`).join(", "));
  out.push("]\n");
  out.push(`${indent(1)}},\n`);
  out.push(`${indent(1)}<span class="k">"results"</span>: {\n`);
  out.push(`${indent(2)}<span class="k">"bindings"</span>: [\n`);
  json.results.bindings.forEach((b, i) => {
    out.push(`${indent(3)}{`);
    const entries = Object.entries(b);
    entries.forEach(([k, v], j) => {
      out.push(` <span class="k">"${k}"</span>: { <span class="k">"type"</span>: <span class="s">"${v.type}"</span>, <span class="k">"value"</span>: <span class="s">"${v.value}"</span> }`);
      if (j < entries.length - 1) out.push(",");
    });
    out.push(` }${i < json.results.bindings.length - 1 ? "," : ""}\n`);
  });
  out.push(`${indent(2)}]\n`);
  out.push(`${indent(1)}}\n`);
  out.push("}\n");
  return <div className="raw" dangerouslySetInnerHTML={{ __html: out.join("") }}/>;
}

// ── Plan ───────────────────────────────────────────────────────────────
function Plan({ rows }) {
  const maxMs = Math.max(...rows.map(r => r.ms));
  return (
    <div className="plan">
      <div className="plan-row h">
        <div>operation</div>
        <div className="num">rows</div>
        <div className="num">time (ms)</div>
        <div className="num">cost</div>
      </div>
      {rows.map((r, i) => (
        <div key={i} className="plan-row">
          <div className="indent" style={{ "--indent": (r.indent * 18) + "px" }}>
            <span className="op">{r.op}</span>
            <span style={{ color: "var(--ink-2)", marginLeft: 8 }}>{r.detail}</span>
          </div>
          <div className="num">{r.rows.toLocaleString()}</div>
          <div className="num">
            {r.ms.toFixed(2)}
            <span className="plan-bar" style={{ width: Math.max(4, (r.ms / maxMs) * 60) + "px" }}/>
          </div>
          <div className="num" style={{ color: "var(--ink-3)" }}>{(r.rows * 0.012).toFixed(1)}</div>
        </div>
      ))}
    </div>
  );
}

// ── Log ────────────────────────────────────────────────────────────────
function LogView({ rows }) {
  return (
    <div className="log">
      {rows.map((r, i) => (
        <div key={i} className="log-row">
          <span className="t">{r.t}</span>
          <span className={"lv " + r.lv}>{r.lv.toUpperCase()}</span>
          <span className="msg">{r.msg}</span>
          <span className="src">{r.src}</span>
        </div>
      ))}
    </div>
  );
}

// ── Bottom panel shell ─────────────────────────────────────────────────
function BottomPanel({ sparql, setSparql, sql, result, running, tab, setTab, height, setHeight }) {
  const D = window.PGRDF_DATA;
  const resizing = useRefB(null);

  // Live result if a query has run; otherwise fall back to fixture rows so
  // the Results/Plan/Log tabs still render something before the first Run.
  const liveCols = result ? result.cols : D.RESULT_COLS;
  const liveRows = result ? result.rows : D.RESULT_ROWS;
  const statusText = running
    ? "running…"
    : result
      ? (result.error
          ? `error · ${result.ms} ms`
          : `${result.ms} ms · ${result.count} row${result.count === 1 ? "" : "s"} · live`)
      : "—";

  const onResizeDown = (e) => {
    e.preventDefault();
    resizing.current = { y: e.clientY, h: height };
    const move = (ev) => {
      const r = resizing.current;
      if (!r) return;
      const next = Math.min(Math.max(120, r.h - (ev.clientY - r.y)), window.innerHeight * 0.7);
      setHeight(next);
    };
    const up = () => {
      resizing.current = null;
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  return (
    <div className="bottom" style={{ height }}>
      <div className="resizer-v" onMouseDown={onResizeDown}/>
      <div className="bottom-tabs">
        <div className={"bottom-tab " + (tab === "sparql" ? "active" : "")} onClick={() => setTab("sparql")}>SPARQL</div>
        <div className={"bottom-tab " + (tab === "sql" ? "active" : "")} onClick={() => setTab("sql")}>generated SQL</div>
        <div className={"bottom-tab " + (tab === "results" ? "active" : "")} onClick={() => setTab("results")}>
          Results <span className="n">{liveRows.length}</span>
        </div>
        <div className={"bottom-tab " + (tab === "jsonld" ? "active" : "")} onClick={() => setTab("jsonld")}>JSON-LD</div>
        <div className={"bottom-tab " + (tab === "plan" ? "active" : "")} onClick={() => setTab("plan")}>Plan</div>
        <div className={"bottom-tab " + (tab === "log" ? "active" : "")} onClick={() => setTab("log")}>Log</div>

        <div className="bottom-tabs-right">
          <span className="dot" style={running ? { background: "var(--accent)" } : {}}/>
          <span>{statusText}</span>
        </div>
      </div>
      <div className="bottom-body">
        {tab === "sparql" && <EditableEditor value={sparql} onChange={setSparql}/>}
        {tab === "sql"    && <Editor value={sql}    lang="sql"/>}
        {tab === "results"&& (
          result && result.error
            ? <div className="results" style={{ padding: 16, color: "var(--err, #c0392b)", fontFamily: "var(--mono)" }}>
                query error: {result.error}
              </div>
            : <Results cols={liveCols} rows={liveRows}/>
        )}
        {tab === "jsonld" && <JsonLd  cols={liveCols} rows={liveRows}/>}
        {tab === "plan"   && <Plan    rows={D.PLAN_ROWS}/>}
        {tab === "log"    && <LogView rows={D.LOG_ROWS}/>}
      </div>
    </div>
  );
}

window.BottomPanel = BottomPanel;
