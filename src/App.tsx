import { BrowserRouter, Routes, Route } from "react-router-dom";
import Home from "./pages/Home";
import Crews from "./pages/Crews";
import CrewEditor from "./pages/CrewEditor";

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Home />} />
        <Route path="/crews" element={<Crews />} />
        <Route path="/crews/:crewId" element={<CrewEditor />} />
      </Routes>
    </BrowserRouter>
  );
}
