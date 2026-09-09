import "./App.css";

const workflowSteps = [
  {
    number: "01",
    title: "Context Loaded",
    status: "Completed",
  },
  {
    number: "02",
    title: "Codex Handoff Ready",
    status: "Ready",
  },
  {
    number: "03",
    title: "Run Validation",
    status: "Next",
  },
];

const projects = ["NEXORA", "SentinelLite AI", "ARM-SecNet", "Portfolio"];

function App() {
  return (
    <main className="ghost-app">
      <section className="ghost-window">
        <header className="ghost-header">
          <div className="brand-area">
            <div className="brand-mark">G</div>
            <div>
              <p className="brand-name">GHOST</p>
              <p className="brand-subtitle">
                Secure Personal AI Workflow Coordinator
              </p>
            </div>
          </div>

          <div className="header-center">
            <span></span>
            <p>COMMAND SPACE</p>
            <span></span>
          </div>

          <div className="secure-status">
            <span className="status-dot"></span>
            LOCAL · SECURE
          </div>
        </header>

        <section className="content-grid">
          <section className="intro-panel">
            <div className="page-kicker">CURRENT SESSION</div>

            <h1>
              Good morning,
              <br />
              Kavisara
            </h1>

            <p className="intro-copy">
              Your secure AI workflow coordinator is ready.
            </p>

            <div className="session-card">
              <p className="card-label">ACTIVE PROJECT</p>
              <h2>NEXORA</h2>
              <p>Add wishlist price alert MVP</p>

              <div className="progress-pill">
                <span></span>
                IN PROGRESS
              </div>
            </div>
          </section>

          <section className="orbit-panel">
            <div className="orbit-core">
              <div className="core-glow"></div>
            </div>

            {projects.map((project, index) => (
              <div key={project} className={`project-node node-${index + 1}`}>
                <span></span>
                {project}
              </div>
            ))}

            <div className="orbit-ring ring-one"></div>
            <div className="orbit-ring ring-two"></div>
          </section>
        </section>

        <section className="workflow-strip">
          <div className="strip-title">NEXT ACTION</div>

          <div className="workflow-steps">
            {workflowSteps.map((step) => (
              <article key={step.number} className="workflow-step">
                <span className="step-number">{step.number}</span>
                <div>
                  <h3>{step.title}</h3>
                  <p>{step.status}</p>
                </div>
              </article>
            ))}
          </div>
        </section>

        <section className="command-bar">
          <span className="command-spark">✦</span>
          <p>Ask GHOST or type a command...</p>
          <button type="button">➜</button>
        </section>

        <footer className="security-footer">
          <span>Local-first</span>
          <span>•</span>
          <span>Secrets protected</span>
          <span>•</span>
          <span>Manual approval required</span>
        </footer>
      </section>
    </main>
  );
}

export default App;
