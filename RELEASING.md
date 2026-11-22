# Как выпустить обновление Hearth

Установленные копии раз в 6 часов (и при запуске) проверяют
`https://github.com/kiqwea/hearth/releases/latest/download/latest.json`.
Если там версия новее, в боковой панели появляется «Доступна версия X · Обновить».
По кнопке Hearth останавливает серверы с сохранением мира, скачивает обновление,
проверяет подпись и перезапускается. Миры, Java и настройки сохраняются.

## Выпуск

1. Описать изменения для пользователей в `RELEASE_NOTES.md` (этот текст увидят в окне обновления).
2. Поднять версию везде сразу:
   ```powershell
   node scripts/bump-version.mjs 0.2.0
   ```
3. Закоммитить, поставить тег и отправить:
   ```powershell
   git commit -am "Hearth 0.2.0"
   git tag v0.2.0
   git push; git push origin v0.2.0
   ```
4. GitHub Actions (workflow `Release`) соберёт установщик, подпишет обновление и опубликует
   релиз с `Hearth_0.2.0_x64-setup.exe` и `latest.json`. Проверить: `gh run list`, `gh release view v0.2.0`.

Версия обязательно должна быть больше предыдущей, иначе обновление не придёт.

Локальная подписанная сборка (обычно не нужна, это делает GitHub):

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = "$env:USERPROFILE\.tauri\hearth-updater.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
npm run tauri build
```

## Ключ подписи обновлений

- Закрытый ключ: `%USERPROFILE%\.tauri\hearth-updater.key` (без пароля). В этот репозиторий он не попадает никогда.
- Для сборки он лежит в секрете репозитория `TAURI_SIGNING_PRIVATE_KEY`.
- Резервная копия: закрытый репозиторий владельца `hearth-keys`.
- Публичный ключ записан в `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`).
- Если ключ потерян, уже установленные копии не смогут принять новые обновления:
  придётся сделать новый ключ, и всем один раз переустановить Hearth вручную.
