import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Rule = { on: boolean; id: number; keep: number; below: number };
type Config = { on: boolean; balls: Rule; revives: Rule; potions: Rule; potionFirst: boolean; goldFloor: number };
type Offer = {
  id: number;
  name: string | null;
  price: number;
  have: number;
  heal?: number;
  healPct?: number;
  cost?: number | null;
  golden?: boolean;
};
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
const GOLDEN = 70070;

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
  { id: GOLDEN, name: "Golden Potion", price: 0, have: 0, healPct: 0.5, golden: true },
];

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const n = (value: number) => Math.round(value).toLocaleString("pt-BR");
const cost = (value: number) => value.toLocaleString("pt-BR", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
// "Todas as poções" não inclui a Golden, que cura por % do HP e só conta quando é a escolhida.
const sum = (offers?: Offer[]) => (offers ?? []).filter((o) => !o.golden).reduce((total, o) => total + o.have, 0);

function potionText(p: Offer): string {
  const price = p.price > 0 ? ` · ${n(p.price)} ouro` : "";
  if (p.golden) return `${p.name}${price} · cura ${Math.round((p.healPct ?? 0.5) * 100)}% do HP`;
  return `${p.name}${price} · ${n(p.heal ?? 0)} HP`;
}

let config: Config | null = null;
let status: Status | null = null;
let fullLog = false;

/** Os interruptores são botões com role="switch"; o estado fica em aria-checked. */
const switches = {
  on: $<HTMLButtonElement>("on"),
  ballsOn: $<HTMLButtonElement>("balls-on"),
  revivesOn: $<HTMLButtonElement>("revives-on"),
  potionsOn: $<HTMLButtonElement>("potions-on"),
};
const isOn = (button: HTMLButtonElement) => button.getAttribute("aria-checked") === "true";
const setSwitch = (button: HTMLButtonElement, on: boolean) => button.setAttribute("aria-checked", String(on));
const potionFirst = $<HTMLInputElement>("potion-first");
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
  setSwitch(switches.on, c.on);
  setSwitch(switches.ballsOn, c.balls.on);
  setSwitch(switches.revivesOn, c.revives.on);
  setSwitch(switches.potionsOn, c.potions.on);
  potionFirst.checked = c.potionFirst;
  counts.ballsKeep.value = n(c.balls.keep);
  counts.ballsBelow.value = n(c.balls.below);
  counts.revivesKeep.value = n(c.revives.keep);
  counts.revivesBelow.value = n(c.revives.below);
  counts.potionsKeep.value = n(c.potions.keep);
  counts.potionsBelow.value = n(c.potions.below);
  counts.goldFloor.value = n(c.goldFloor);
}

/** A barra do item: quanto tem, até onde mantém, e a marca do "comprar abaixo de". */
function showStock(kind: "balls" | "revives" | "potions", rule: Rule, have: number | null) {
  $(`${kind}-have`).replaceChildren("Na mochila ", Object.assign(document.createElement("strong"), { textContent: have === null ? "—" : n(have) }));
  const low = have !== null && have < rule.below;
  $(`${kind}-rule`).textContent = !rule.on
    ? "não recompra"
    : have === null
      ? `mantém ${n(rule.keep)}, compra abaixo de ${n(rule.below)}`
      : low
        ? `abaixo do limite: comprando até ${n(rule.keep)}`
        : `ok até cair abaixo de ${n(rule.below)}`;
  const scale = Math.max(rule.keep, rule.below, have ?? 0, 1);
  const meter = $(`${kind}-meter`);
  meter.classList.toggle("low", low);
  meter.querySelector<HTMLElement>(".fill")!.style.width = `${have === null ? 0 : (have / scale) * 100}%`;
  meter.querySelector<HTMLElement>(".mark")!.style.left = `calc(${(rule.below / scale) * 100}% - 1px)`;
  meter.role = "img";
  meter.ariaLabel = have === null ? "Estoque ainda não lido" : `Na mochila ${n(have)} de ${n(rule.keep)}; compra abaixo de ${n(rule.below)}`;
  meter.closest(".item")!.classList.toggle("disabled", !rule.on);
}

function showStatus() {
  if (!config) return;
  const s = status;
  const connected = !!s?.connected;
  const golden = config.potions.id === GOLDEN;

  const banner = $("banner");
  banner.className = `banner ${!config.on ? "" : connected && !s?.paused ? "ok" : "warn"}`;
  $("banner-title").textContent = config.on ? "Recompra ligada" : "Recompra desligada";
  $("state").textContent = !config.on
    ? "Ligada, ela confere o estoque a cada segundo e compra o que faltar."
    : !connected
      ? "Esperando o jogo conectar: a conta precisa estar no jogo, não na tela de login."
      : s?.paused
        ? "Em pausa: 3 compras seguidas não deram certo. Volta sozinha em 2 min; veja o registro."
        : `Comprando${s?.nick ? ` na conta ${s.nick}` : ""} · confere a cada segundo`;
  $("gold").textContent = connected && s ? n(s.gold ?? 0) : "—";

  const balls = s?.balls?.length ? s.balls : KNOWN_BALLS;
  fill(ball, balls.map((b) => ({ id: b.id, text: b.price > 0 ? `${b.name} · ${n(b.price)} ouro` : b.name ?? `bola ${b.id}` })), config.balls.id);
  const potions = s?.potions?.length ? s.potions : KNOWN_POTIONS;
  fill(potion, [{ id: BEST_VALUE, text: "Melhor custo-benefício (automático)" }, ...potions.map((p) => ({ id: p.id, text: potionText(p) }))], config.potions.id);
  $("potions-keep-label").textContent = golden ? "Manter (só Golden)" : "Manter";

  const chosenBall = connected ? s?.balls?.find((b) => b.id === config!.balls.id) : undefined;
  showStock("balls", config.balls, connected ? chosenBall?.have ?? 0 : null);
  showStock("revives", config.revives, connected ? sum(s?.revives) : null);
  const goldenOffer = s?.potions?.find((p) => p.golden);
  showStock("potions", config.potions, connected ? (golden ? goldenOffer?.have ?? 0 : sum(s?.potions)) : null);

  const revives = s?.revives ?? [];
  $("revives-info").textContent =
    connected && revives.length ? `${revives.map((r) => `${r.name} ${n(r.have)}`).join(" · ")}\nCompra: ${revives[0].name}, ${n(revives[0].price)} ouro` : "";

  const lines: string[] = [];
  if (connected && s) {
    if (golden) {
      lines.push(goldenOffer ? "Golden Potion: conta só ela, por quantidade." : "A Golden Potion não está à venda no NPC.");
    } else if (s.maxHp != null && s.threshold != null) {
      const costs = (s.potions ?? []).filter((p) => p.cost != null && !p.golden).sort((a, b) => a.cost! - b.cost!);
      lines.push(
        `Ouro por HP útil no pokémon ativo (${n(s.maxHp)} HP, poção abaixo de ${Math.round(s.threshold * 100)}%): ` +
          costs.map((p) => `${p.name} ${cost(p.cost!)}`).join(" · "),
      );
    }
    const chosen = s.potions?.find((p) => p.id === s.potionId);
    if (chosen) lines.push(`Compra: ${chosen.name}${s.queueFirst === chosen.id ? " (1ª na fila do “Usar Poções”)" : ""}`);
  }
  $("potions-info").textContent = lines.join("\n");
  for (const id of ["revives-info", "potions-info"]) $(id).hidden = !$(id).textContent;

  const all = s?.log ?? [];
  const shown = fullLog ? all : all.filter((line) => line.includes(" comprou ")).slice(-4);
  const log = $("log");
  const text = shown.length ? shown.join("\n") : fullLog ? "Nada no registro ainda." : "Nenhuma compra ainda.";
  log.classList.toggle("full", fullLog);
  if (log.textContent !== text) {
    log.textContent = text;
    log.scrollTop = log.scrollHeight;
  }
  $("log-title").textContent = fullLog ? "Registro" : "Últimas compras";
  $("log-toggle").textContent = fullLog ? "Mostrar só as compras" : "Ver registro completo";
}

function read(): Config {
  return {
    on: isOn(switches.on),
    balls: { on: isOn(switches.ballsOn), id: Number(ball.value), keep: readCount(counts.ballsKeep, MAX_STOCK), below: readCount(counts.ballsBelow, MAX_STOCK) },
    revives: { on: isOn(switches.revivesOn), id: 0, keep: readCount(counts.revivesKeep, MAX_STOCK), below: readCount(counts.revivesBelow, MAX_STOCK) },
    potions: { on: isOn(switches.potionsOn), id: Number(potion.value), keep: readCount(counts.potionsKeep, MAX_STOCK), below: readCount(counts.potionsBelow, MAX_STOCK) },
    potionFirst: potionFirst.checked,
    goldFloor: readCount(counts.goldFloor, MAX_GOLD),
  };
}

async function apply() {
  config = read();
  showStatus();
  await invoke("restock_set", { config });
}

for (const button of Object.values(switches)) {
  button.addEventListener("click", () => {
    setSwitch(button, !isOn(button));
    apply();
  });
}
potionFirst.addEventListener("change", apply);
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
$("log-toggle").addEventListener("click", () => {
  fullLog = !fullLog;
  showStatus();
});

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
