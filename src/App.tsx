import { ThemePref, useTheme } from "./lib/theme";
import { useEngineStatus } from "./lib/useEngineStatus";
import { EngineGate } from "./components/EngineGate";
import { ModelSetup } from "./components/ModelSetup";
import { Workspace } from "./components/Workspace";
import { ext } from "./ext";

export default function App() {
  const [theme, setTheme] = useTheme();
  return (
    <ext.Gate>
      <Main theme={theme} setTheme={setTheme} />
    </ext.Gate>
  );
}

function Main({ theme, setTheme }: { theme: ThemePref; setTheme: (t: ThemePref) => void }) {
  const { status: engine, everReady, retry, apply: applyEngine } = useEngineStatus(true);

  // Without a chat model there is nothing to chat with: setup, also when the model went missing
  // later. Otherwise full-screen gates only until the engine has been ready once; after that the
  // workspace stays mounted and reports engine trouble in a banner.
  if (engine?.state === "needs_model") return <ModelSetup />;
  if ((!everReady || !engine) && engine?.state !== "ready") return <EngineGate status={engine} onRetry={retry} />;
  return <Workspace engine={engine} onEngineRetry={retry} onEngine={applyEngine} theme={theme} setTheme={setTheme} />;
}
