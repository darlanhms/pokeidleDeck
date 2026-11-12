import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const EMPTY = "Nenhum resgate ainda. Escolha o bônus de cada conta em Lives → Config…; o baú da Twitch vem ligado de fábrica.";
const pre = document.getElementById("lines")!;
let lines: string[] = [];

function render() {
  pre.textContent = lines.length ? lines.join("\n") : EMPTY;
}

invoke<{ lines: string[] }>("registro_get").then((view) => {
  lines = view.lines;
  render();
});
listen<string>("registro-line", (e) => {
  lines.unshift(e.payload);
  render();
});
document.getElementById("open-file")!.addEventListener("click", () => invoke("registro_open_file"));
