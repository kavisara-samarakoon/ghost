import "./App.css";

/**
 * GHOST Command Space — single-screen frontend.
 *
 * Static mock data only. No AI API calls, no network requests, no shell
 * execution, no file scanning, no .env reads, no CLI integration.
 */

/* -----------------------------------------------------------------------
   Orbit visual (CSS + inline SVG, no external images)
   ----------------------------------------------------------------------- */

function OrbitVisual() {
  return (
    <div className="orbit-container" aria-hidden="true">
      {/* Concentric orbit rings */}
      <div className="orbit-ring orbit-ring-1" />
      <div className="orbit-ring orbit-ring-2" />
      <div className="orbit-ring orbit-ring-3" />

      {/* Central glowing core */}
      <div className="orbit-core">
        <div className="orbit-core-inner" />
      </div>

      {/* Project nodes at cardinal positions */}
      <div className="orbit-node orbit-node-top orbit-node-active">
        <div className="orbit-node-dot" />
        <span className="orbit-node-label">NEXORA</span>
      </div>

      <div className="orbit-node orbit-node-right">
        <div className="orbit-node-dot" />
        <span className="orbit-node-label">SentinelLite AI</span>
      </div>

      <div className="orbit-node orbit-node-bottom">
        <div className="orbit-node-dot" />
        <span className="orbit-node-label">ARM-SecNet</span>
      </div>

      <div className="orbit-node orbit-node-left">
        <div className="orbit-node-dot" />
        <span className="orbit-node-label">Portfolio</span>
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
   Inline SVG icons (no external dependencies)
   ----------------------------------------------------------------------- */

function SearchIcon() {
  return (
    <svg className="command-bar-icon" viewBox="0 0 24 24" fill="none"
      stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"
      strokeLinejoin="round" aria-hidden="true">
      <circle cx="11" cy="11" r="7" />
      <path d="m16 16 5 5" />
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

function ChevronIcon() {
  return (
    <svg width="10" height="10" viewBox="0 0 24 24" fill="none"
      stroke="currentColor" strokeWidth="2" strokeLinecap="round"
      strokeLinejoin="round" aria-hidden="true">
      <path d="m9 6 6 6-6 6" />
    </svg>
  );
}

/* -----------------------------------------------------------------------
   Next action steps
   ----------------------------------------------------------------------- */

const actionSteps = [
  { number: "01", label: "Run validation" },
  { number: "02", label: "Review Codex output" },
  { number: "03", label: "Prepare update pack" },
] as const;

function ActionRow() {
  return (
    <div className="action-row" role="list" aria-label="Next actions">
      {actionSteps.map((step, index) => (
        <div
          key={step.number}
          className={`action-step${index === 0 ? " action-step-active" : ""}`}
          role="listitem"
        >
          <span className="action-step-number">{step.number}</span>
          <span className="action-step-text">{step.label}</span>
          <span className="action-step-arrow">
            <ChevronIcon />
          </span>
        </div>
      ))}
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
        <div className="top-bar-left">
          <div className="window-dots" aria-hidden="true">
            <span className="window-dot red" />
            <span className="window-dot yellow" />
            <span className="window-dot green" />
          </div>
          <div className="brand-text">
            <span className="brand-name">GHOST</span>
            <span className="brand-separator" />
            <span>Command Space</span>
          </div>
        </div>
        <div className="top-bar-right">
          <span className="status-dot" />
          <span>Local</span>
          <span>•</span>
          <span>Secure</span>
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
          <SearchIcon />
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
        <span className="safety-text">
          Local-first · Secrets protected · Manual approval required
        </span>
      </div>
    </div>
  );
}

export default App;
