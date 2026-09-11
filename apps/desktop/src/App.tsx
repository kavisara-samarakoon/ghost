import "./App.css";

/**
 * GHOST Command Space — single-screen frontend.
 *
 * Static mock data only. No AI API calls, no network requests, no shell
 * execution, no file scanning, no .env reads, no CLI integration.
 */

/* -----------------------------------------------------------------------
   Orbit visual — SVG rings, spokes, and nodes
   ----------------------------------------------------------------------- */

function OrbitVisual() {
  const cx = 210;
  const cy = 210;
  const r1 = 60;   /* inner dashed ring */
  const r2 = 110;  /* middle ring */
  const r3 = 160;  /* outer ring */

  /* Four node positions: top, right, bottom, left */
  const nodes = [
    { x: cx, y: cy - r3 + 10 },  /* top — NEXORA */
    { x: cx + r3 - 10, y: cy },  /* right — ARM-SecNet */
    { x: cx, y: cy + r3 - 10 },  /* bottom — Portfolio */
    { x: cx - r3 + 10, y: cy },  /* left — SentinelLite AI */
  ];

  return (
    <div className="orbit-container" aria-hidden="true">
      <svg className="orbit-svg" viewBox="0 0 420 420">
        {/* Outer orbit ring */}
        <circle cx={cx} cy={cy} r={r3} fill="none"
          stroke="rgba(77,216,232,0.06)" strokeWidth="1" />

        {/* Middle orbit ring */}
        <circle cx={cx} cy={cy} r={r2} fill="none"
          stroke="rgba(77,216,232,0.08)" strokeWidth="1" />

        {/* Inner dashed orbit ring */}
        <circle cx={cx} cy={cy} r={r1} fill="none"
          stroke="rgba(77,216,232,0.1)" strokeWidth="1"
          strokeDasharray="4 6" />

        {/* Crosshair lines through center */}
        <line x1={cx} y1={cy - r3 - 8} x2={cx} y2={cy + r3 + 8}
          stroke="rgba(77,216,232,0.05)" strokeWidth="1" />
        <line x1={cx - r3 - 8} y1={cy} x2={cx + r3 + 8} y2={cy}
          stroke="rgba(77,216,232,0.05)" strokeWidth="1" />

        {/* Spoke lines from center to each node */}
        {nodes.map((node, i) => (
          <line key={i} x1={cx} y1={cy} x2={node.x} y2={node.y}
            stroke="rgba(77,216,232,0.06)" strokeWidth="1" />
        ))}

        {/* Small tick marks on outer ring at 45° angles */}
        {[45, 135, 225, 315].map((angle) => {
          const rad = (angle * Math.PI) / 180;
          const ix = cx + (r3 - 6) * Math.cos(rad);
          const iy = cy + (r3 - 6) * Math.sin(rad);
          const ox = cx + (r3 + 6) * Math.cos(rad);
          const oy = cy + (r3 + 6) * Math.sin(rad);
          return (
            <line key={angle} x1={ix} y1={iy} x2={ox} y2={oy}
              stroke="rgba(77,216,232,0.1)" strokeWidth="1" />
          );
        })}
      </svg>

      {/* Central dark orb with core dot */}
      <div className="orbit-core" />
      <div className="orbit-core-ring" />
      <div className="orbit-core-dot" />

      {/* Project nodes */}
      <div className="orbit-node orbit-node-top orbit-node-active">
        <div className="orbit-node-dot" />
        <span className="orbit-node-label">NEXORA</span>
      </div>

      <div className="orbit-node orbit-node-right">
        <div className="orbit-node-dot" />
        <span className="orbit-node-label">ARM-SecNet</span>
      </div>

      <div className="orbit-node orbit-node-bottom">
        <div className="orbit-node-dot" />
        <span className="orbit-node-label">Portfolio</span>
      </div>

      <div className="orbit-node orbit-node-left">
        <div className="orbit-node-dot" />
        <span className="orbit-node-label">SentinelLite AI</span>
      </div>

      {/* Status info labels */}
      <span className="orbit-info orbit-info-1 orbit-info-active">
        Context loaded
      </span>
      <span className="orbit-info orbit-info-2">
        Codex handoff ready
      </span>
      <span className="orbit-info orbit-info-3">
        Validation pending
      </span>
    </div>
  );
}

/* -----------------------------------------------------------------------
   Inline SVG icons
   ----------------------------------------------------------------------- */

function SparkleIcon() {
  return (
    <svg className="command-bar-icon" viewBox="0 0 24 24" fill="none"
      stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"
      strokeLinejoin="round" aria-hidden="true">
      <path d="M12 2v4m0 12v4M4.93 4.93l2.83 2.83m8.48 8.48 2.83 2.83M2 12h4m12 0h4M4.93 19.07l2.83-2.83m8.48-8.48 2.83-2.83" />
    </svg>
  );
}

function SendIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"
      aria-hidden="true">
      <path d="M5 12h14m-6-6 6 6-6 6" />
    </svg>
  );
}

/* -----------------------------------------------------------------------
   Next action steps (timeline style)
   ----------------------------------------------------------------------- */

const actionSteps = [
  { number: "01", label: "Run validation" },
  { number: "02", label: "Review Codex output" },
  { number: "03", label: "Prepare update pack" },
] as const;

function ActionRow() {
  return (
    <div className="next-action-area">
      <div className="next-action-label">Next Action</div>
      <div className="action-row" role="list" aria-label="Next actions">
        {actionSteps.map((step, index) => (
          <div key={step.number} style={{ display: "contents" }}>
            {index > 0 && <div className="action-connector" />}
            <div
              className={`action-step${index === 0 ? " action-step-active" : ""}`}
              role="listitem"
            >
              <span className="action-step-number">{step.number}</span>
              <span className="action-step-text">{step.label}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/* -----------------------------------------------------------------------
   Main App
   ----------------------------------------------------------------------- */

function App() {
  return (
    <div className="command-space">
      {/* ---- Top bar ---- */}
      <div className="top-bar">
        <div className="window-dots" aria-hidden="true">
          <span className="window-dot red" />
          <span className="window-dot yellow" />
          <span className="window-dot green" />
        </div>

        <div className="brand-area">
          <span className="brand-name">GHOST</span>
        </div>

        <div className="top-rule" />
        <span className="top-title">COMMAND SPACE</span>
        <div className="top-rule-right" />

        <div className="top-status">
          <span className="status-dot" />
          <span>LOCAL • SECURE</span>
        </div>
      </div>

      {/* ---- Main content: greeting + orbit ---- */}
      <div className="main-content">
        <div className="left-panel">
          <div className="greeting">
            <h1>
              Good evening,
              <br />
              Kavisara
            </h1>
            <p className="greeting-subtitle">
              Your secure AI workflow coordinator is ready.
            </p>
          </div>

          <div className="session-block">
            <span className="session-label">Current Session</span>
            <span className="session-project">NEXORA</span>
            <span className="session-goal">
              Add wishlist price alert MVP
            </span>
            <span className="status-pill">
              <span className="status-pill-dot" />
              In Progress
            </span>
          </div>
        </div>

        <div className="right-panel">
          <OrbitVisual />
        </div>
      </div>

      {/* ---- Next action row ---- */}
      <ActionRow />

      {/* ---- Command bar ---- */}
      <div className="command-bar-area">
        <div className="command-bar">
          <SparkleIcon />
          <input
            className="command-input"
            type="text"
            placeholder="Ask GHOST or type a command..."
            aria-label="Command input"
            readOnly
          />
          <button className="command-send" type="button" aria-label="Send">
            <SendIcon />
          </button>
        </div>
      </div>

      {/* ---- Footer safety line ---- */}
      <div className="safety-footer">
        <span className="safety-rule" />
        <span className="safety-text">
          Local-first &nbsp;•&nbsp; Secrets protected &nbsp;•&nbsp; Manual approval required
        </span>
        <span className="safety-rule" />
      </div>
    </div>
  );
}

export default App;
