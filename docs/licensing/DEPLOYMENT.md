# Jarvis License Server: развёртывание и эксплуатация

Сервер — один исполняемый файл (`jarvis-license-server`) и база SQLite. Он слушает только локальный адрес; HTTPS даёт обратный прокси перед ним. **Сервер в этом репозитории не развёрнут и домена нет: это ваш шаг (см. «Что должен сделать владелец»).**

## 1. Сборка и первый запуск
```bash
cargo build --release -p jarvis-license-server
sudo useradd --system --home /var/lib/jarvis-license jarvis-license
sudo install -d -m 700 -o jarvis-license /etc/jarvis-license /var/lib/jarvis-license /var/backups/jarvis-license
sudo cp deploy/license-server.env.example /etc/jarvis-license/server.env && sudo chmod 600 /etc/jarvis-license/server.env
set -a; . /etc/jarvis-license/server.env; set +a
sudo -E -u jarvis-license ./target/release/jarvis-license-server init          # база, тарифы, два файла секретов; печатает ПУБЛИЧНЫЙ ключ
sudo -E -u jarvis-license ./target/release/jarvis-license-server admin-create owner   # токен администратора jla_… показывается ОДИН раз
```
Секреты (`signing.key`, `pepper.key`) создаются с правами 600, **не попадают в репозиторий**. Сделайте их офлайн-копию: без `pepper.key` выданные ключи станут неизвестны серверу, без `signing.key` придётся выпускать новую сборку клиента.

## 2. Служба (systemd)
```ini
# /etc/systemd/system/jarvis-license.service
[Unit]
Description=Jarvis License Server
After=network-online.target
[Service]
User=jarvis-license
EnvironmentFile=/etc/jarvis-license/server.env
ExecStart=/usr/local/bin/jarvis-license-server serve
Restart=on-failure
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/jarvis-license
PrivateTmp=true
[Install]
WantedBy=multi-user.target
```

## 3. HTTPS (обязательно)
Пример Caddy (сертификат выдаётся сам):
```
license.example.com {
    reverse_proxy 127.0.0.1:8787
}
```
Клиент принимает только `https://` (для проверки на своём компьютере разрешён `http://127.0.0.1`). Если прокси ставит реальный адрес клиента в заголовок, укажите его в `JARVIS_LS_CLIENT_IP_HEADER`, иначе все запросы выглядят как один адрес (ограничения на подбор ключей сработают на всех сразу).

## 4. Сборка Jarvis User с вашим сервером
В настройках репозитория GitHub → Settings → Variables → Actions задайте (это не секреты, это публичные значения):
* `JARVIS_LICENSE_URL` — `https://license.example.com`
* `JARVIS_LICENSE_PUBKEY` — вывод `jarvis-license-server public-key`

Шаг `build-user` в `.github/workflows/build-windows.yml` соберёт `Jarvis-User-Setup.exe`. Без этих двух значений программа соберётся, но активировать Premium не сможет (CI предупредит).
Локально: `JARVIS_LICENSE_URL=… JARVIS_LICENSE_PUBKEY=… cargo build --release -p jarvis-app --features user-edition`.

## 5. Ежедневная работа
* **Выдать ключи**: Jarvis Admin → «Лицензии» → «Создать ключи» (тариф или свой срок от 7 дней / в календарных месяцах, цена, дата начала, срок на активацию, заметка, количество). Полные ключи показываются один раз: сервер хранит только их отпечатки.
* **Продлить клиента**: создайте ключ типа «Ключ продления», отправьте клиенту; срок добавится к концу текущей подписки (после окончания — начнётся с момента продления). Повторный ввод того же ключа ничего не меняет.
* **Заблокировать / отозвать / сбросить привязку / продлить вручную / сменить тариф / срок активации** — меню «Действие» в списке; каждое действие попадает в журнал (кто, когда, что было → что стало).
* **Цены и тарифы** — таблица «Тарифы и цены»: цена, валюта, включён/выключен. Стандарт: Monthly 1 мес. $5, 6 Months $25, Yearly 12 мес. $50, Lifetime — цена назначается вручную.

## 6. Резервные копии и восстановление
```bash
jarvis-license-server backup /var/backups/jarvis-license/ls-$(date +%F).sqlite     # целая согласованная копия в НОВЫЙ файл
```
Запускайте по расписанию (cron/systemd timer) и храните копии вне сервера. Восстановление: остановить службу → положить копию как `JARVIS_LS_DB` (права 600) → вернуть `signing.key` и `pepper.key` → запустить. Проверка: `curl https://license.example.com/health` → `{"ok":true,…}`.

## 7. Ротация ключей и инциденты
* **Утёк токен администратора**: создайте нового (`admin-create`) — старый перестаёт работать только после удаления из базы (`admins`); журнал покажет, что делал токен.
* **Утёк `signing.key`**: сгенерируйте новую пару (новая база не нужна: остановите службу, удалите файл, `init` создаст новый, **публичный ключ изменится**), соберите новый Jarvis User с новым `JARVIS_LICENSE_PUBKEY` и разошлите клиентам. Старые токены новая сборка не принимает; клиент снова вводит свой ключ — повторная активация **той же установкой** безопасна и ничего не пересчитывает.
* **Утёк `pepper.key`**: хеши ключей можно перебирать только при знании pepper и базы одновременно; перевыпустите ключи (отзовите старые).
* **Клиент сменил компьютер**: «Сбросить привязку» в списке → клиент вводит тот же ключ на новом компьютере (срок не пересчитывается).
* **Ложные срабатывания по подбору ключей**: ограничение сбрасывается само через несколько минут.

## 8. Что должен сделать владелец (я этого сделать не могу)
1. Арендовать сервер и домен, настроить HTTPS (раздел 3).
2. Выполнить `init`, сохранить секреты офлайн, создать администратора.
3. Задать переменные `JARVIS_LICENSE_URL` / `JARVIS_LICENSE_PUBKEY` в репозитории и собрать Jarvis User.
4. Подключить приём оплаты (платёжный провайдер) — в программе и на сервере **оплата не реализована**; сейчас клиент платит вам вне программы и получает ключ продления.
5. Настроить резервные копии (раздел 6) и проверить восстановление.


## 9. Быстрый путь: Docker (папка `deploy/docker/`)
Если у вас есть сервер с Docker и домен, направленный на него:
```bash
cd deploy/docker
echo "LICENSE_DOMAIN=license.example.com" > .env
docker compose up -d --build
docker compose exec license jarvis-license-server public-key          # -> JARVIS_LICENSE_PUBKEY (переменная репозитория)
docker compose exec license jarvis-license-server admin-create owner  # -> токен администратора (виден один раз)
```
Первый запуск сам делает `init` (база, тарифы, файлы секретов в томе `/data`). Наружу открыты только порты Caddy 80/443 (HTTPS выдаётся автоматически), сам сервер доступен только внутри сети compose. Резервная копия: `docker compose exec license jarvis-license-server backup /data/ls-$(date +%F).sqlite`, затем скопируйте файл **и** `signing.key`, `pepper.key` из тома с сервера. CI на каждый коммит собирает этот образ и проверяет, что он поднимается, создаёт базу и отвечает на `/health` (задача `license-server-image` в `build-windows.yml`). **Само развёртывание на вашем сервере — ваш шаг: ни сервера, ни домена в репозитории нет.**
