# Fase 1: spike de viabilidade (macOS 26.7, Tauri 2.11.6, wry 0.55.1)

## Como rodar

```
npm install
npm run tauri dev
DECK_KICK_SPIKE=1 DECK_KICK_CHANNEL=<canal> npm run tauri dev   # + os 3 players de teste da Kick
```

Autoteste das contas (usa uma pasta de dados separada; rode a fase 2 depois da 1, com a mesma pasta):

```
cd src-tauri
DECK_SELFTEST=1 DECK_DATA_DIR=/tmp/deck-teste cargo run
DECK_SELFTEST=2 DECK_DATA_DIR=/tmp/deck-teste cargo run
```

Fase 2 (layouts, ampliar, zoom, desligar, nome, reordenar, remover/restaurar): `DECK_SELFTEST=shell`.
Fase 3 (menos animações, eco, ULTRA, recompra): `DECK_SELFTEST=auto`.

O resultado fica em `$DECK_DATA_DIR/selftest-<fase>.txt`. O log da Kick fica em `kick-spike.log`, na pasta de
dados (`~/Library/Application Support/com.darlanhermes.pokeidle-deck`).

## Resultados

| Item | Resultado | Como |
|---|---|---|
| 1. Contas isoladas por UUID | OK (autoteste: 27 checagens) | `with_webview_configuration` + `WKWebsiteDataStore(forIdentifier:)` |
| 1. Login sobrevive a reabrir | OK | cookie + localStorage voltam na fase 2 |
| 1. Remover não afeta as outras | OK | as outras seguem com a própria marca |
| 1. Layout 1/2/3/4 sem buraco | OK | `layout::grid` + testes unitários |
| 2. Script no documento + IPC | OK | `initialization_script` (recompra.js original + report.js) e capability `remote` só com `allow-account-report` |
| 3. JS com Promise | OK | `callAsyncJavaScript` via objc2 |
| 4. Kick 10+ min fora da tela | OK, 27/27 checagens em 13 min, nas 3 variantes | ver abaixo |
| 5. Menu nativo sobre as webviews | OK | `popup_menu_at` (NSMenu) |

## Kick: o que foi preciso

Sem tudo isto o player não toca oculto:

1. `mediaTypesRequiringUserActionForPlayback = none` (só dá passando uma `WKWebViewConfiguration` própria).
2. `visible.js`: a página se vê sempre `visible`. Sem isso o player nem anexa o stream (`readyState 0`) e pausa
   com `AbortError` assim que a janela fica coberta.
3. `WKPreferences` privadas `_pageVisibilityBasedProcessSuppressionEnabled = NO` e
   `_hiddenPageDOMTimerThrottlingEnabled = NO`. Sem isso o player para em poucos minutos e a página deixa de
   responder. `inactiveSchedulingPolicy = .none` sozinho não basta.
4. Mudo por `_setPageMuted:` (privada) e por JS.

Com isso, as 3 formas de esconder funcionaram: janela sem borda fora da tela, a mesma sem detecção de oclusão, e
webview filha oculta dentro da janela principal.

## Achados que mudam a Fase 2

- **O wry nunca libera a WKWebView que fecha** (`Drop` faz `retain()` de propósito). Consequências: o WebKit
  recusa apagar o data store ("Data store is in use") até o app fechar, e a página removida continuaria
  rodando. Contorno: `_close` (privada) na página ao remover, limpar o conteúdo do data store na hora e apagar a
  pasta no início seguinte (`pendingDelete` no deck.json).
- **A janela do Tauri tem conteúdo de tamanho cheio**: a barra de título fica por cima, e o WKWebView embaixo
  dela se desloca sozinho. O layout usa `contentLayoutRect` para começar abaixo da barra.
- APIs privadas usadas: `_close`, `_setPageMuted:`, `_setWindowOcclusionDetectionEnabled:`,
  `_pageVisibilityBasedProcessSuppressionEnabled`, `_hiddenPageDOMTimerThrottling*`. Tudo checado com
  `respondsToSelector:` antes. Impede App Store, não impede distribuição direta.
