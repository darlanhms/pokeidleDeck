import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type UiState = { name: string; tabs: { title: string; selected: boolean }[]; revealed: boolean; summary: string };

const tabs = document.getElementById("tabs")!;
const empty = document.getElementById("empty")!;

function render(state: UiState | null) {
  if (!state) return;
  empty.textContent = state.summary || "Nenhuma live aberta agora.";
  empty.hidden = state.tabs.length > 0;
  tabs.replaceChildren(
    ...state.tabs.map((tab, index) => {
      const button = Object.assign(document.createElement("button"), {
        type: "button",
        textContent: tab.title,
        className: tab.selected ? "current" : "",
      });
      if (tab.selected) button.ariaCurrent = "page";
      button.addEventListener("click", () => invoke("lives_ui_select", { index }));
      return button;
    }),
  );
}

document.getElementById("park")!.addEventListener("click", () => invoke("lives_ui_park"));
listen<UiState | null>("lives-ui", (e) => render(e.payload));
invoke<UiState | null>("lives_ui_state").then(render);
