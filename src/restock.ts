import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Rule = { on: boolean; id: number; keep: number; below: number };
type Config = { on: boolean; balls: Rule; revives: Rule; potions: Rule; potionFirst: boolean; goldFloor: number };
type Offer = { id: number; name: string | null; price: number; have: number; heal?: number; cost?: number | null };
type Status = {
  configured: boolean;
  on: boolean;
  connected: boolean;
  nick?: string | null;
  gold?: number;
  balls?: Offer[];
  revives?: Offer[];
  potions?: Offer[];
  potionId?: number | null;
  maxHp?: number | null;
  threshold?: number | null;
  queueFirst?: number | null;
  paused?: boolean;
  log?: string[];
};
type Info = { name: string; config: Config; status: Status | null };

const MAX_STOCK = 10_000_000;
const MAX_GOLD = 1_000_000_000_000;
const BEST_VALUE = 0;

// Antes de a página contar o que tem: os nomes do catálogo do jogo, para a janela já abrir preenchida.
const KNOWN_BALLS: Offer[] = [
  { id: 1, name: "Poke Ball", price: 0, have: 0 },
  { id: 2, name: "Great Ball", price: 0, have: 0 },
  { id: 3, name: "Super Ball", price: 0, have: 0 },
  { id: 4, name: "Ultra Ball", price: 0, have: 0 },
];
const KNOWN_POTIONS: Offer[] = [
  { id: 200, name: "Small Potion", price: 50, have: 0, heal: 60 },
  { id: 201, name: "Great Potion", price: 150, have: 0, heal: 150 },
  { id: 202, name: "Ultra Potion", price: 400, have: 0, heal: 400 },
  { id: 203, name: "Hyper Potion", price: 800, have: 0, heal: 1000 },
  { id: 204, name: "Ultimate Potion", price: 1500, have: 0, heal: 3000 },
];

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const n = (value: number) => Math.round(value).toLocaleString("pt-BR");
const cost = (value: number) => value.toLocaleString("pt-BR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const sum = (offers?: Offer[]) => (offers ?? []).reduce((total, o) => total + o.have, 0);

let config: Config | null = null;
let status: Status | null = null;

const checks = { on: $<HTMLInputElement>("on"), ballsOn: $<HTMLInputElement>("balls-on"), revivesOn: $<HTMLInputElement>("revives-on"), potionsOn: $<HTMLInputElement>("potions-on"), potionFirst: $<HTMLInputElement>("potion-first") };
const counts = {
  ballsKeep: $<HTMLInputElement>("balls-keep"),
  ballsBelow: $<HTMLInputElement>("balls-below"),
  revivesKeep: $<HTMLInputElement>("revives-keep"),
  revivesBelow: $<HTMLInputElement>("revives-below"),
  potionsKeep: $<HTMLInputElement>("potions-keep"),
  potionsBelow: $<HTMLInputElement>("potions-below"),
  goldFloor: $<HTMLInputElement>("gold-floor"),
};
const ball = $<HTMLSelectElement>("ball");
const potion = $<HTMLSelectElement>("potion");

// Quantidade inteira escrita como no registro (30.000); aceita digitar com ponto, vírgula ou sem nada.
function readCount(input: HTMLInputElement, max: number): number {
  const digits = input.value.replace(/\D/g, "");
  return Math.min(max, digits ? Number(digits) : 0);
}

function fill(select: HTMLSelectElement, options: { id: number; text: string }[], selected: number) {
  if (!options.some((o) => o.id === selected)) options.push({ id: selected, text: `item ${selected}` });
  const same =
    select.options.length === options.length &&
    options.every((o, i) => select.options[i].value === String(o.id) && select.options[i].text === o.text);
  if (!same && document.activeElement !== select) {
    select.replaceChildren(...options.map((o) => new Option(o.text, String(o.id))));
  }
  if (document.activeElement !== select) select.value = String(selected);
}

function showConfig(c: Config) {
  checks.on.checked = c.on;
  checks.ballsOn.checked = c.balls.on;
  checks.revivesOn.checked = c.revives.on;
  checks.potionsOn.checked = c.potions.on;
  checks.potionFirst.checked = c.potionFirst;
  counts.ballsKeep.value = n(c.balls.keep);
  counts.ballsBelow.value = n(c.balls.below);
  counts.revivesKeep.value = n(c.revives.keep);
  counts.revivesBelow.value = n(c.revives.below);
  counts.potionsKeep.value = n(c.potions.keep);
  counts.potionsBelow.value = n(c.potions.below);
  counts.goldFloor.value = n(c.goldFloor);
}

function showStatus() {
  if (!config) return;
  const s = status;
  const connected = !!s?.connected;
  const state = $("state");
  state.className = `state ${!config.on ? "" : connected && !s?.paused ? "ok" : "warn"}`;
  state.textContent = !config.on
    ? "Desligada. Ligada, ela confere o estoque a cada segundo e compra o que faltar."
    : !connected
      ? "Ligada, esperando o jogo conectar (a conta precisa estar no jogo, não na tela de login)."
      : s?.paused
        ? "Ligada, em pausa: 3 compras seguidas não deram certo (volta sozinha em 2 min; veja o registro)."
        : `Ligada${s?.nick ? ` na conta ${s.nick}` : ""}.`;

  const balls = s?.balls?.length ? s.balls : KNOWN_BALLS;
  fill(ball, balls.map((b) => ({ id: b.id, text: b.price > 0 ? `${b.name} · ${n(b.price)} ouro` : b.name ?? `bola ${b.id}` })), config.balls.id);
  const potions = s?.potions?.length ? s.potions : KNOWN_POTIONS;
  fill(
    potion,
    [{ id: BEST_VALUE, text: "Melhor custo-benefício (automático)" }, ...potions.map((p) => ({ id: p.id, text: `${p.name} · ${n(p.price)} ouro · ${n(p.heal ?? 0)} HP` }))],
    config.potions.id,
  );

  if (!connected || !s) {
    $("balls-info").textContent = $("revives-info").textContent = $("potions-info").textContent = "Na mochila: —";
    $("gold").textContent = "";
  } else {
    const chosenBall = s.balls?.find((b) => b.id === config!.balls.id);
    $("balls-info").textContent = chosenBall ? `Na mochila: ${n(chosenBall.have)}` : "Essa pokébola não está à venda no NPC.";
    const revives = s.revives ?? [];
    $("revives-info").textContent =
      `Na mochila: ${n(sum(revives))}` +
      (revives.length ? ` (${revives.map((r) => `${r.name} ${n(r.have)}`).join(" · ")})\nCompra: ${revives[0].name}, ${n(revives[0].price)} ouro` : "");
    const lines = [`Na mochila (todas): ${n(sum(s.potions))}`];
    if (s.maxHp != null && s.threshold != null) {
      const costs = (s.potions ?? []).filter((p) => p.cost != null).sort((a, b) => a.cost! - b.cost!);
      lines.push(
        `Ouro por HP útil no pokémon ativo (${n(s.maxHp)} HP, poção abaixo de ${Math.round(s.threshold * 100)}%): ` +
          costs.map((p) => `${p.name} ${cost(p.cost!)}`).join(" · "),
      );
    } else {
      lines.push("Custo-benefício: esperando o jogo mandar o pokémon ativo.");
    }
    const chosen = s.potions?.find((p) => p.id === s.potionId);
    if (chosen) lines.push(`Compra: ${chosen.name}${s.queueFirst === chosen.id ? ' (1ª na fila do "Usar Poções")' : ""}`);
    $("potions-info").textContent = lines.join("\n");
    $("gold").textContent = `ouro agora: ${n(s.gold ?? 0)}`;
  }

  const log = $("log");
  const text = (s?.log ?? []).join("\n");
  if (log.textContent !== text) {
    log.textContent = text;
    log.scrollTop = log.scrollHeight;
  }
}

function read(): Config {
  return {
    on: checks.on.checked,
    balls: { on: checks.ballsOn.checked, id: Number(ball.value), keep: readCount(counts.ballsKeep, MAX_STOCK), below: readCount(counts.ballsBelow, MAX_STOCK) },
    revives: { on: checks.revivesOn.checked, id: 0, keep: readCount(counts.revivesKeep, MAX_STOCK), below: readCount(counts.revivesBelow, MAX_STOCK) },
    potions: { on: checks.potionsOn.checked, id: Number(potion.value), keep: readCount(counts.potionsKeep, MAX_STOCK), below: readCount(counts.potionsBelow, MAX_STOCK) },
    potionFirst: checks.potionFirst.checked,
    goldFloor: readCount(counts.goldFloor, MAX_GOLD),
  };
}

async function apply() {
  config = read();
  showStatus();
  await invoke("restock_set", { config });
}

for (const box of Object.values(checks)) box.addEventListener("change", apply);
for (const select of [ball, potion]) select.addEventListener("change", apply);
for (const input of Object.values(counts)) {
  input.inputMode = "numeric";
  input.addEventListener("change", async () => {
    await apply();
    showConfig(config!);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") input.blur();
  });
}

listen<Status | null>("restock-status", (e) => {
  status = e.payload;
  showStatus();
});

invoke<Info | null>("restock_get").then((info) => {
  if (!info) return;
  document.title = `Recompra · ${info.name}`;
  config = info.config;
  status = info.status;
  showConfig(config);
  showStatus();
});
