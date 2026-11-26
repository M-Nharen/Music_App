# Connecting Spotify to Qwertone

Qwertone talks to Spotify through **your own personal Spotify developer app**.
It takes about five minutes, once. You need a **Spotify Premium** account
(Spotify requires Premium for developer apps in Development Mode, and for
playback control).

What Qwertone does with it: shows what's playing, searches, plays / pauses /
seeks, opens the practice version of the song ("Learn this"), moves the
falling notes and lyrics along with Spotify (sync mode), and looks up lyrics
by Spotify's title / artist / length. Spotify's *audio* is never used: practice
versions are made from your own copy of the song (mp3, flac…) or a MIDI file.

## 1. Create the app

1. Open <https://developer.spotify.com/dashboard> and log in with your
   Spotify account. The first time, accept the developer terms (and confirm
   your e-mail if Spotify asks).
2. Press **Create app** and fill in:
   - **App name**: anything, e.g. `Qwertone (personal)`.
   - **App description**: anything, e.g. `Practise songs on my keyboard`.
   - **Website**: leave empty.
   - **Redirect URIs**: type exactly

     ```
     http://127.0.0.1:8898/callback
     ```

     and press **Add**. It must be `127.0.0.1`, not `localhost` (Spotify
     doesn't allow `localhost` any more), with `http`, the port `8898` and
     `/callback`, nothing else.
   - **Which API/SDKs are you planning to use?**: tick **Web API**.
3. Tick the box to agree to the terms and press **Save**.
4. On the app's page open **Settings** and copy the **Client ID** (32 letters
   and digits). You do **not** need the client secret: Qwertone logs in with
   PKCE, which works without one. Don't paste the secret anywhere.

Your own account can use the app straight away. If you ever log in to
Qwertone with a *different* Spotify account, add that account's e-mail under
the app's **User Management** first (up to five people).

## 2. Log in from Qwertone

1. Open the **Spotify** tab.
2. Paste the Client ID and press **Log in with Spotify**.
3. Your browser opens Spotify's page: press **Agree**. You'll see
   *“Logged in to Spotify”*; close the tab and go back to Qwertone.

If no browser opens, press **Copy the link** and paste it into your browser.
The login waits five minutes for you.

### Where the login is kept

- In your **desktop keyring** (GNOME Keyring, KeePassXC, or anything that
  offers the Secret Service), through `secret-tool`:

  ```sh
  sudo pacman -S libsecret gnome-keyring   # if you don't have a keyring yet
  ```

  On Hyprland the keyring has to be started with your session; if
  `secret-tool search application qwertone` says it can't reach the Secret
  Service, add `exec-once = gnome-keyring-daemon --start --components=secrets`
  to your Hyprland config.
- If no keyring answers, in `~/.local/share/qwertone/spotify-login.json`,
  readable only by you (permissions 600, folder 700).

`config.toml` only has the Client ID (not a secret). **Spotify settings →
Keep the login in** lets you force the keyring or the file. **Log out**
deletes the login from both; to cut Qwertone off completely, also remove it
at <https://www.spotify.com/account/apps/>.

## 3. Using it

- **Now playing**: what Spotify plays on any of your devices, with a
  progress bar (click to jump), play / pause and ±10 s.
- **Search**: press Enter to search (Spotify allows 10 results per search for
  personal apps). Searches are remembered for a week, so the same search costs
  nothing; while Spotify is paused you still get songs you've seen before.
- **Learn this**: opens the practice version if you've analysed the song.
  If not, Qwertone looks for your copy in `~/Music` and `~/Downloads` (by
  file name) and offers to analyse it, or you pick the file. Once it's done
  the Spotify track and the song are linked, so next time it's one click.
- **Sync mode** (Learn this on the song that's playing): the Song tab follows
  Spotify. The falling notes, the chord sheet, the lyrics and the scoring move
  with Spotify's position; Qwertone plays no backing (Spotify is the sound).
  Play / pause / seek in the Song tab control Spotify. Speed, loops and wait
  mode are off while following (Spotify doesn't wait); press **Stop
  following** to practise slowly. If the notes come a little before or after
  the music, use **Notes − later / + earlier** (saved per song: different
  releases of a song often start a few hundred ms apart). Easy parts moved to
  another key are swapped for the original-key part, so you're in tune with
  the recording.
- **Lyrics** for whatever plays, synced to Spotify: from your analysed song if
  it has them, otherwise looked up on LRCLIB by Spotify's title, artist,
  album and length (once per track). With the analyzer installed they get
  English letters (Tanglish / Tenglish / Hinglish) and the English
  translation like any song; without it they show in their own script.
  By default only the **English letters** are shown; turn on *Original
  script* or *English* if you want them.

## Spotify's limits, and what Qwertone does about them

Personal (Development Mode) apps have small, unpublished rate limits and a
quota. Qwertone keeps its calls few:

- It asks what's playing **only while the Spotify tab or sync mode is on
  screen**, every 12 s (Spotify settings: 10–60 s), plus once right after you
  press play / pause / seek and once when a track should have ended. In
  between it works out the position itself (last position + time passed).
  A minimised window asks nothing.
- Searches and track details are cached on disk
  (`~/.cache/qwertone/spotify.json`), lyrics per track in
  `~/.cache/qwertone/spotify/lyrics/`.
- It never sends more than 16 calls in any 30 s (background checks stop at
  11, leaving room for your clicks), and nothing is ever retried in a loop.
- If Spotify answers **429** (too many requests, or `QUOTA_EXCEEDED`), every
  call stops until the time Spotify gives in `Retry-After` (an hour if it
  says nothing about the quota), longer if it happens again. The header shows
  **“Spotify paused for N min”**. Meanwhile the app keeps working: sync mode
  carries on from the last known position, search shows songs seen before,
  lyrics already found stay. Use the Spotify app itself to control playback
  until the pause is over.
- If Spotify can't be reached, Qwertone waits 15 s, then 30 s, … up to 5 min
  between tries (**Try again now** in Spotify settings skips the wait).

## Troubleshooting

- **“INVALID_CLIENT: Invalid redirect URI”** in the browser: the Redirect URI
  in the app's settings isn't exactly `http://127.0.0.1:8898/callback`.
- **“Couldn't listen on 127.0.0.1:8898”**: another program uses that port.
  Set another port in `~/.config/qwertone/config.toml`:

  ```toml
  [spotify]
  redirect_port = 8899
  ```

  and add `http://127.0.0.1:8899/callback` to the app's Redirect URIs.
- **“Spotify refused to play … Premium is needed”** or **403**: the account
  isn't Premium, or (another account) isn't in the app's User Management.
- **“Spotify isn't open on any device”**: open Spotify (desktop app, phone,
  or <https://open.spotify.com>), or pick a device from the list Qwertone
  shows.
- **Lyrics a bit early / late**: the *timing* box under the lyrics. Bluetooth
  delay is already taken into account when Spotify plays on this computer.
- **Logged out by itself**: Spotify ended the login (password change, app
  removed at spotify.com/account/apps, or another Client ID). Log in again.
