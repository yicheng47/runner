import { useEffect } from "react";
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { AppShell } from "./components/AppShell";
import { UpdateProvider } from "./contexts/UpdateContext";
import Crews from "./pages/Crews";
import CrewEditor from "./pages/CrewEditor";
import MissionWorkspace from "./pages/MissionWorkspace";
import Runners from "./pages/Runners";
import RunnerDetail from "./pages/RunnerDetail";
import RunnerChat from "./pages/RunnerChat";

export default function App() {
  // Tell the backend the first frame has painted so it can show + focus the
  // main window. Don't wrap this in requestAnimationFrame — macOS pauses rAF
  // for hidden windows, so the rAF callback would never fire and the window
  // would stay hidden forever. useEffect runs after React commits, which is
  // enough to guarantee a non-blank webview before show.
  useEffect(() => {
    invoke("app_ready").catch(console.error);
  }, []);

  return (
    <UpdateProvider>
      <BrowserRouter>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/" element={<Navigate to="/runners" replace />} />
            <Route path="/crews" element={<Crews />} />
            <Route path="/crews/:crewId" element={<CrewEditor />} />
            <Route path="/runners" element={<Runners />} />
            <Route path="/runners/:handle" element={<RunnerDetail />} />
            <Route path="/runners/:handle/chat/:sessionId" element={<RunnerChat />} />
            <Route path="/missions/:id" element={<MissionWorkspace />} />
            <Route path="*" element={<Navigate to="/runners" replace />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </UpdateProvider>
  );
}
