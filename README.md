# PokeIdle Deck

Até 4 contas do PokeIdle lado a lado, com recompra, eco, ULTRA e as lives da Twitch e da Kick para o bônus.
Port para macOS do [PokeIdleMulti](https://github.com/artbruno35/PokeIdleMulti).

macOS 14 ou mais novo, Apple Silicon e Intel.

## Instalar

1. Baixe o `PokeIdle Deck_<versão>_universal.dmg` na página de [Releases](https://github.com/darlanhms/pokeidleDeck/releases).
2. Abra o `.dmg` e arraste o **PokeIdle Deck** para a pasta **Aplicativos**.
3. Abra o app. Na primeira vez o macOS bloqueia (veja abaixo).

### "Abrir Mesmo Assim"

O app não é assinado nem notarizado pela Apple, então na primeira vez o macOS diz que não pode verificar o
desenvolvedor e não abre.

1. Abra o app uma vez e feche o aviso.
2. Vá em **Ajustes do Sistema → Privacidade e Segurança**.
3. Role até a mensagem sobre o **PokeIdle Deck** e clique em **Abrir Mesmo Assim**.
4. Confirme com a senha ou o Touch ID.

Das próximas vezes ele abre direto.

Se o macOS disser que o app "está danificado", tire a quarentena pelo Terminal:

```
xattr -cr "/Applications/PokeIdle Deck.app"
```

## Desenvolver

```
npm install
npm run tauri dev
```

Testes: `npm test` (scripts do jogo) e `cargo test` em `src-tauri`. Autotestes que abrem o app: ver `SPIKE.md`.

## Lançar uma versão

A versão sai da tag. Ao subir uma tag `v*`, o GitHub Actions (`.github/workflows/release.yml`) roda os testes,
gera o `.dmg` universal e publica num Release com o nome da tag.

```
git tag v0.2.0
git push origin v0.2.0
```

Para gerar o mesmo `.dmg` aqui:

```
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri build -- --target universal-apple-darwin --bundles app,dmg
```

O `.dmg` fica em `src-tauri/target/universal-apple-darwin/release/bundle/dmg/`.
