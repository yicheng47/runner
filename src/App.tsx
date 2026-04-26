import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import Crews from "./pages/Crews";
import CrewEditor from "./pages/CrewEditor";
import Debug from "./pages/Debug";
import Runners from "./pages/Runners";
import RunnerDetail from "./pages/RunnerDetail";
import RunnerChat from "./pages/RunnerChat";

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Navigate to="/crews" replace />} />
        <Route path="/crews" element={<Crews />} />
        <Route path="/crews/:crewId" element={<CrewEditor />} />
        <Route path="/runners" element={<Runners />} />
        <Route path="/runners/:handle" element={<RunnerDetail />} />
        <Route path="/runners/:handle/chat/:sessionId" element={<RunnerChat />} />
        {/* Scratch page for C6 PTY validation — remove when C10 lands. */}
        <Route path="/debug" element={<Debug />} />
      </Routes>
    </BrowserRouter>
  );
}
