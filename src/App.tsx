import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { UpdateProvider } from "./contexts/UpdateContext";
import Crews from "./pages/Crews";
import CrewEditor from "./pages/CrewEditor";
import MissionWorkspace from "./pages/MissionWorkspace";
import Runners from "./pages/Runners";
import RunnerDetail from "./pages/RunnerDetail";
import RunnerChat from "./pages/RunnerChat";

export default function App() {
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
            <Route path="/runners/:handle/chat" element={<RunnerChat />} />
            <Route path="/missions/:id" element={<MissionWorkspace />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </UpdateProvider>
  );
}
