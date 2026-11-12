import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type UiState = {
  name: string;
  tabs: { title: string; selected: boolean }[];
  revealed: boolean;
  summary: string;
  points: boolean;
  fullPage: boolean;
  canPoints: boolean;
};

const tabs = document.getElementById("tabs")!;
const empty = document.getElementById("empty")!;

function render(state: UiState | null) {
  if (!state) return;
  empty.textContent = state.summary || "Nenhuma live aberta agora.";
  empty.hidden = state.tabs.length > 0;
  const points = document.getElementById("points") as HTMLButtonElement;
  points.disabled = !state.canPoints;
  points.classList.toggle("active", state.points);
  points.ariaPressed = String(state.points);
  const full = document.getElementById("full-page") as HTMLButtonElement;
  full.hidden = !state.points;
  full.classList.toggle("active", state.fullPage);
  full.ariaPressed = String(state.fullPage);
  tabs.replaceChildren(
    ...state.tabs.map((tab, index) => {
      const button = Object.assign(document.createElement("button"), {
        type: "button",
        textContent: tab.title,
        className: tab.selected && !state.points ? "current" : "",
      });
      if (tab.selected) button.ariaCurrent = "page";
      button.addEventListener("click", () => invoke("lives_ui_select", { index }));
      return button;
    }),
  );
}

document.getElementById("park")!.addEventListener("click", () => invoke("lives_ui_park"));
document.getElementById("points")!.addEventListener("click", () => invoke("lives_ui_points"));
document.getElementById("full-page")!.addEventListener("click", () => invoke("lives_ui_full_page"));
listen<UiState | null>("lives-ui", (e) => render(e.payload));
invoke<UiState | null>("lives_ui_state").then(render);
