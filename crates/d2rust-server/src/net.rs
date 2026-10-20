//! The network side: the QUIC endpoint's connections and their sessions
//! (contracts sections 3 and 6).
//!
//! Each connection's control stream opens with the handshake
//! ([`ClientHello`], checked against this server: protocol, game, build,
//! tick rate; then the session limits), answered with [`SessionWelcome`]
//! or [`Refused`]. Then it carries the session's requests:
//!
//! - kinds 0x20-0x3F: accounts and characters, answered here on the
//!   [`Accounts`] API (a new login to an account that has a session takes
//!   it over: the old one gets `SessionTaken`, its character leaves its
//!   game saved, and it is closed);
//! - kinds 0x40-0x5F: the lobby ([`LobbyApi::handle`]), once a character
//!   is selected;
//! - anything else: `NotSupported`; a request that doesn't decode is
//!   `Malformed` and ends the connection.
//!
//! The client's next stream is its game stream: `Attach` with the lobby's
//! join ticket ([`LobbyApi::redeem`]) seats the session's character in the
//! game ([`crate::games::GameHandle::admit`]); `Attach` with the game's
//! resume token takes a held slot back.
//!
//! A lost connection of a logged-in session keeps the session for the
//! grace period, so a client that comes back with the session token in its
//! hello finds it as it was ([`ResumeState`]). The games' events update the
//! sessions (the game a session is in, its character's new summary), save
//! the summaries ([`Accounts::saved`]), then go to the lobby.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use d2_net::msg::account::{AccountEvent, AccountRequest};
use d2_net::msg::{kind as msg_kind, ErrorCode};
use d2_net::wire::{Attach, ClientHello, Refused, ResumeState, ServerInfo, SessionWelcome};
use d2_net::ResumeToken;
use d2_quic::{FramedStream, PacketStream, QuicConn, QuicServer};
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use crate::accounts::{Account, AccountId, Accounts, NewChar};
use crate::games::{GameEvent, Games};
use crate::log::ServerLog;
use crate::session::{ControlOut, ControlTx, LobbyApi, SessionCtx, SessionId, Sessions};

/// How long a client has for its hello, and a game stream for its attach.
pub const HANDSHAKE_WAIT: Duration = Duration::from_secs(10);

/// How long a connection stays open after its last frame (a refusal, a
/// goodbye) so the frame arrives: closing a QUIC connection drops what it
/// hasn't delivered yet.
pub const CLOSE_GRACE: Duration = Duration::from_secs(1);

/// Sessions not logged in yet, per address (contracts section 5).
pub const UNAUTHENTICATED_PER_IP: usize = 4;

/// What the network side runs with.
#[derive(Clone, Debug)]
pub struct NetConfig {
    /// What a client's hello must match.
    pub server: ServerInfo,
    pub server_name: String,
    /// The most players of one game (for the welcome).
    pub max_players: u32,
    pub max_sessions: usize,
    /// How long a lost session is kept for its client to come back.
    pub grace: Duration,
}

type Ctx = Arc<AsyncMutex<SessionCtx>>;

/// The network side of the server.
pub struct Net<L: LobbyApi> {
    cfg: NetConfig,
    accounts: Accounts,
    lobby: Arc<L>,
    games: Games,
    sessions: Sessions,
    log: Arc<ServerLog>,
    state: Mutex<State>,
    next_id: AtomicU64,
}

#[derive(Default)]
struct State {
    /// Every session's context, open or held.
    ctxs: BTreeMap<SessionId, Ctx>,
    /// Session tokens, for a hello's resume.
    tokens: BTreeMap<[u8; 16], SessionId>,
    /// Sessions whose connection was lost, with when they end.
    held: BTreeMap<SessionId, Instant>,
    /// Sessions not logged in, per address.
    unauthenticated: BTreeMap<IpAddr, usize>,
    /// The account's session: claimed atomically by a login, so two
    /// logins at once cannot both keep it.
    by_account: BTreeMap<AccountId, SessionId>,
}

/// A refused hello or attach.
type Refusal = Refused;

impl<L: LobbyApi> Net<L> {
    pub fn new(
        cfg: NetConfig,
        accounts: Accounts,
        lobby: Arc<L>,
        games: Games,
        sessions: Sessions,
        log: Arc<ServerLog>,
    ) -> Self {
        Self {
            cfg,
            accounts,
            lobby,
            games,
            sessions,
            log,
            state: Mutex::new(State::default()),
            next_id: AtomicU64::new(1),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open sessions (not counting held ones).
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Accepts connections until the endpoint closes.
    pub async fn run(self: Arc<Self>, quic: Arc<QuicServer>) {
        while let Some((conn, control)) = quic.accept().await {
            let me = self.clone();
            tokio::spawn(async move { me.serve(conn, control).await });
        }
    }

    /// Takes the games' events until the games are gone.
    pub async fn run_events(self: Arc<Self>, mut rx: mpsc::Receiver<GameEvent>) {
        while let Some(ev) = rx.recv().await {
            self.game_event(&ev).await;
            self.lobby.on_game_event(ev);
        }
    }

    /// Ends the held sessions whose grace period ran out. The server calls
    /// it every second.
    pub async fn expire_held(&self) {
        let now = Instant::now();
        let over: Vec<SessionId> = self
            .state()
            .held
            .iter()
            .filter(|(_, &until)| until <= now)
            .map(|(&id, _)| id)
            .collect();
        for id in over {
            self.end_session(id, "its connection was lost").await;
        }
    }

    /// One connection: the handshake, then the session.
    async fn serve(self: Arc<Self>, conn: QuicConn, mut control: FramedStream) {
        let ip = conn.peer().ip();
        let hello = match tokio::time::timeout(HANDSHAKE_WAIT, control.recv_async()).await {
            Ok(Ok(body)) => body,
            _ => {
                conn.close(0, "no hello");
                return;
            }
        };
        let opened =
            match ClientHello::parse(&hello).and_then(|h| h.check(&self.cfg.server).map(|()| h)) {
                Ok(h) => self.open_session(ip, h.resume).await,
                Err(r) => Err(r),
            };
        let (ctx, rx, welcome) = match opened {
            Ok(x) => x,
            Err(r) => {
                let _ = control.send(r.body());
                control.close();
                close_later(conn, "refused");
                return;
            }
        };
        let id = ctx.lock().await.id;
        if control.send(welcome.body()).is_err() {
            self.lose_session(id).await;
            return;
        }
        let streams = {
            let me = self.clone();
            let (conn, ctx) = (conn.clone(), ctx.clone());
            tokio::spawn(async move { me.game_streams(conn, ctx).await })
        };
        let clean = self.session_loop(&ctx, &mut control, rx).await;
        streams.abort();
        if clean {
            control.close();
            close_later(conn, "bye");
            self.end_session(id, "logged out").await;
        } else {
            self.lose_session(id).await;
        }
    }

    /// Reads requests and writes answers until the session ends. True when
    /// it ended cleanly (logout, takeover, malformed request), false when
    /// the connection was lost.
    async fn session_loop(
        &self,
        ctx: &Ctx,
        control: &mut FramedStream,
        mut rx: mpsc::UnboundedReceiver<ControlOut>,
    ) -> bool {
        loop {
            tokio::select! {
                got = control.recv_async() => match got {
                    Ok(body) => self.dispatch(ctx, body).await,
                    Err(_) => return false,
                },
                out = rx.recv() => match out {
                    Some(ControlOut::Body(b)) => {
                        if control.send(b).is_err() {
                            return false;
                        }
                    }
                    Some(ControlOut::Close) | None => {
                        // Send what was queued before the close.
                        while let Ok(ControlOut::Body(b)) = rx.try_recv() {
                            let _ = control.send(b);
                        }
                        return true;
                    }
                },
            }
        }
    }

    /// A new session, or a held one taken back with its token.
    #[allow(clippy::type_complexity)]
    async fn open_session(
        &self,
        ip: IpAddr,
        resume: Option<ResumeToken>,
    ) -> Result<(Ctx, mpsc::UnboundedReceiver<ControlOut>, SessionWelcome), Refusal> {
        let token = ResumeToken::random();
        // A held session comes back with its state.
        let held = {
            let mut st = self.state();
            match resume.and_then(|t| st.tokens.get(&t.0).copied()) {
                Some(id) if st.held.remove(&id).is_some() => {
                    st.tokens.retain(|_, s| *s != id);
                    st.tokens.insert(token.0, id);
                    st.ctxs.get(&id).cloned().map(|ctx| (id, ctx))
                }
                _ => None,
            }
        };
        if let Some((id, ctx)) = held {
            {
                let (tx, rx) = ControlTx::new(id);
                self.sessions.insert(tx.clone());
                let mut c = ctx.lock().await;
                c.out = tx;
                c.ip = ip;
                let state = c.account.as_ref().map(|(_, name)| ResumeState {
                    account: name.clone(),
                    character: c.character.as_ref().map(|s| s.name.clone()),
                    game_id: c.game,
                });
                drop(c);
                self.log
                    .line(&format!("session {id}: back from {ip} (resumed)"));
                return Ok((ctx, rx, self.welcome(token, state)));
            }
        }
        let mut st = self.state();
        if self.sessions.len() >= self.cfg.max_sessions {
            return Err(Refused::new(ErrorCode::SERVER_FULL));
        }
        let waiting = st.unauthenticated.get(&ip).copied().unwrap_or(0);
        if waiting >= UNAUTHENTICATED_PER_IP {
            return Err(Refused::with_text(
                ErrorCode::SERVER_FULL,
                "Too many connections from your address. Try again later.",
            ));
        }
        *st.unauthenticated.entry(ip).or_default() += 1;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = ControlTx::new(id);
        let ctx = Arc::new(AsyncMutex::new(SessionCtx {
            id,
            ip,
            account: None,
            character: None,
            game: None,
            out: tx.clone(),
        }));
        st.ctxs.insert(id, ctx.clone());
        st.tokens.insert(token.0, id);
        drop(st);
        self.sessions.insert(tx);
        Ok((ctx, rx, self.welcome(token, None)))
    }

    fn welcome(&self, session: ResumeToken, resume: Option<ResumeState>) -> SessionWelcome {
        SessionWelcome {
            tick_rate: self.cfg.server.tick_rate,
            max_players: self.cfg.max_players,
            session,
            server_name: self.cfg.server_name.clone(),
            registration_open: self.accounts.config().registration_open,
            resume,
        }
    }

    /// A lost connection: a logged-in session is held for the grace
    /// period; any other ends now.
    async fn lose_session(&self, id: SessionId) {
        self.sessions.remove(id);
        let Some(ctx) = self.state().ctxs.get(&id).cloned() else {
            return;
        };
        let logged_in = ctx.lock().await.account.is_some();
        if logged_in && !self.cfg.grace.is_zero() {
            self.state()
                .held
                .insert(id, Instant::now() + self.cfg.grace);
        } else {
            self.end_session(id, "its connection was lost").await;
        }
    }

    /// Ends a session for good: its character leaves its game (saved), the
    /// lobby forgets it.
    async fn end_session(&self, id: SessionId, why: &str) {
        self.sessions.remove(id);
        let ctx = {
            let mut st = self.state();
            st.held.remove(&id);
            st.tokens.retain(|_, s| *s != id);
            st.ctxs.remove(&id)
        };
        let Some(ctx) = ctx else {
            return;
        };
        let c = ctx.lock().await;
        if c.account.is_none() {
            self.unauthenticated_done(c.ip);
        }
        if let Some((account, _)) = &c.account {
            let mut st = self.state();
            if st.by_account.get(account) == Some(&id) {
                st.by_account.remove(account);
            }
        }
        if let (Some(g), Some(ch)) = (c.game, c.character.as_ref()) {
            if let Some(h) = self.games.get(g) {
                let (h, name) = (h, ch.name.clone());
                let _ = tokio::task::spawn_blocking(move || h.kick(&name)).await;
            }
        }
        self.lobby.session_ended(&c);
        if let Some((_, name)) = &c.account {
            self.log
                .line(&format!("session {id} ({name}) ended: {why}"));
        }
    }

    fn unauthenticated_done(&self, ip: IpAddr) {
        let mut st = self.state();
        if let Some(n) = st.unauthenticated.get_mut(&ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                st.unauthenticated.remove(&ip);
            }
        }
    }

    /// One request of a session.
    async fn dispatch(&self, ctx: &Ctx, body: Vec<u8>) {
        let kind = body[0];
        if msg_kind::ACCOUNT.contains(&kind) {
            self.account_request(ctx, kind, &body).await;
        } else if msg_kind::LOBBY.contains(&kind) {
            let mut c = ctx.lock().await;
            if c.account.is_none() {
                c.out.error(kind, ErrorCode::NOT_LOGGED_IN, 0, "");
            } else if c.character.is_none() {
                c.out.error(kind, ErrorCode::NO_CHAR_SELECTED, 0, "");
            } else if let Err(d) = self.lobby.handle(&mut c, &body).await {
                c.out.send(d.to_msg(kind).body());
            }
        } else {
            ctx.lock()
                .await
                .out
                .error(kind, ErrorCode::NOT_SUPPORTED, 0, "");
        }
    }

    /// Kinds 0x20-0x3F on the accounts.
    async fn account_request(&self, ctx: &Ctx, kind: u8, body: &[u8]) {
        // The session lock is taken for the state read, and never held
        // across an account call or another session's lock: two logins
        // taking each other over must not deadlock.
        let (req, account, ip, out) = {
            let c = ctx.lock().await;
            let Ok(req) = AccountRequest::parse(body) else {
                c.out.error(kind, ErrorCode::MALFORMED, 0, "");
                c.out.close();
                return;
            };
            (
                req,
                c.account.as_ref().map(|(a, _)| *a),
                c.ip,
                c.out.clone(),
            )
        };
        if !req.before_login() && account.is_none() {
            out.error(kind, ErrorCode::NOT_LOGGED_IN, 0, "");
            return;
        }
        let answer: Result<AccountEvent, crate::accounts::Denied> = match req {
            AccountRequest::Register {
                account: name,
                password,
            } => {
                if account.is_some() {
                    Err(ErrorCode::OTHER.into())
                } else {
                    match self.accounts.register(&name, &password, ip).await {
                        Ok(a) => Ok(self.logged_in(ctx, a).await),
                        Err(d) => Err(d),
                    }
                }
            }
            AccountRequest::Login {
                account: name,
                password,
            } => {
                if account.is_some() {
                    Err(ErrorCode::OTHER.into())
                } else {
                    match self.accounts.login(&name, &password, ip).await {
                        Ok(a) => Ok(self.logged_in(ctx, a).await),
                        Err(d) => Err(d),
                    }
                }
            }
            AccountRequest::Logout => {
                out.close();
                return;
            }
            AccountRequest::CharList => {
                let a = account.expect("checked");
                Ok(AccountEvent::CharList(self.accounts.characters(a).await))
            }
            AccountRequest::CharCreate {
                name,
                class,
                hardcore,
                expansion,
            } => {
                let a = account.expect("checked");
                let new = NewChar {
                    name,
                    class,
                    hardcore,
                    expansion,
                    progression: 0,
                };
                self.accounts
                    .create_character(a, new)
                    .await
                    .map(AccountEvent::CharCreated)
            }
            AccountRequest::CharDelete { name, password } => {
                let a = account.expect("checked");
                if self.character_in_game(&name) {
                    Err(ErrorCode::CHAR_IN_GAME.into())
                } else {
                    match self.accounts.delete_character(a, &name, &password).await {
                        Ok(()) => {
                            let mut c = ctx.lock().await;
                            if c.character
                                .as_ref()
                                .is_some_and(|s| s.name.eq_ignore_ascii_case(&name))
                            {
                                c.character = None;
                            }
                            Ok(AccountEvent::CharDeleted { name })
                        }
                        Err(d) => Err(d),
                    }
                }
            }
            AccountRequest::CharSelect { name } => {
                let a = account.expect("checked");
                let in_game = ctx.lock().await.game.is_some();
                if in_game {
                    Err(ErrorCode::ALREADY_IN_GAME.into())
                } else if self.character_in_game(&name) {
                    Err(ErrorCode::CHAR_IN_GAME.into())
                } else {
                    match self.accounts.character(a, &name).await {
                        Ok(summary) => {
                            ctx.lock().await.character = Some(summary.clone());
                            Ok(AccountEvent::CharSelected(summary))
                        }
                        Err(d) => Err(d),
                    }
                }
            }
        };
        match answer {
            Ok(ev) => out.send(ev.body()),
            Err(d) => out.send(d.to_msg(kind).body()),
        }
    }

    fn character_in_game(&self, name: &str) -> bool {
        let key = name.to_ascii_lowercase();
        self.lobby.character_in_game(&key) || self.games.character_in_game(&key)
    }

    /// A login: the account's session is claimed atomically and the one it
    /// had is taken over, and this one counts as logged in. Sessions are
    /// locked one at a time (never while holding this session's lock), so
    /// two logins cannot deadlock and only one keeps the account.
    async fn logged_in(&self, ctx: &Ctx, a: Account) -> AccountEvent {
        let (id, ip) = {
            let c = ctx.lock().await;
            (c.id, c.ip)
        };
        self.unauthenticated_done(ip);
        let previous = {
            let mut st = self.state();
            st.by_account.insert(a.id, id)
        };
        if let Some(previous) = previous.filter(|&p| p != id) {
            let other = self.state().ctxs.get(&previous).cloned();
            if let Some(other) = other {
                let out = { other.lock().await.out.clone() };
                out.error(msg_kind_login(), ErrorCode::SESSION_TAKEN, 0, "");
                out.close();
                self.end_session(previous, "the account logged in elsewhere")
                    .await;
            }
        }
        self.log
            .line(&format!("session {id}: {} logged in", a.name));
        ctx.lock().await.account = Some((a.id, a.name.clone()));
        AccountEvent::LoggedIn {
            account: a.name,
            max_characters: self.accounts.config().max_characters,
        }
    }

    /// The session's game streams, one at a time.
    async fn game_streams(self: Arc<Self>, conn: QuicConn, ctx: Ctx) {
        while let Some(stream) = conn.accept_stream().await {
            let me = self.clone();
            let ctx = ctx.clone();
            tokio::spawn(async move { me.attach(stream, ctx).await });
        }
    }

    /// A game stream's `Attach`.
    async fn attach(&self, mut stream: FramedStream, ctx: Ctx) {
        let first = tokio::time::timeout(HANDSHAKE_WAIT, stream.recv_async()).await;
        let attach = match first {
            Ok(Ok(body)) => Attach::parse(&body),
            _ => return,
        };
        let refuse = |stream: &mut FramedStream, code: ErrorCode| {
            let _ = stream.send(Refused::new(code).body());
            stream.close();
        };
        match attach {
            Err(_) => refuse(&mut stream, ErrorCode::MALFORMED),
            Ok(Attach::Ticket(ticket)) => {
                let redeemed = {
                    let mut c = ctx.lock().await;
                    if c.game.is_some() {
                        Err(ErrorCode::ALREADY_IN_GAME.into())
                    } else {
                        let r = self.lobby.redeem(&c, ticket);
                        if let Ok((h, _)) = &r {
                            c.game = Some(h.id());
                        }
                        r
                    }
                };
                let (handle, seat) = match redeemed {
                    Ok(x) => x,
                    Err(d) => return refuse(&mut stream, d.code),
                };
                let game = PacketStream::server(stream);
                let admitted = tokio::task::spawn_blocking(move || handle.admit(seat, game)).await;
                if !matches!(admitted, Ok(Ok(()))) {
                    ctx.lock().await.game = None;
                }
            }
            Ok(Attach::Resume(token)) => {
                let game = ctx.lock().await.game;
                let Some(handle) = game.and_then(|g| self.games.get(g)) else {
                    return refuse(&mut stream, ErrorCode::UNKNOWN_RESUME);
                };
                let game = PacketStream::server(stream);
                let _ = tokio::task::spawn_blocking(move || handle.resume(token, game)).await;
            }
        }
    }

    /// The sessions' part of a game event.
    async fn game_event(&self, ev: &GameEvent) {
        let ctxs: Vec<Ctx> = self.state().ctxs.values().cloned().collect();
        match ev {
            GameEvent::Left {
                game,
                account,
                character,
            } => {
                for ctx in ctxs {
                    let mut c = ctx.lock().await;
                    let mine = c.account.as_ref().is_some_and(|(a, _)| a == account)
                        && c.game == Some(*game)
                        && c.character
                            .as_ref()
                            .is_some_and(|s| s.name.eq_ignore_ascii_case(character));
                    if mine {
                        c.game = None;
                    }
                }
            }
            GameEvent::Closed { game, .. } => {
                for ctx in ctxs {
                    let mut c = ctx.lock().await;
                    if c.game == Some(*game) {
                        c.game = None;
                    }
                }
            }
            GameEvent::Saved {
                account, summary, ..
            } => {
                self.accounts.saved(*account, summary.clone()).await;
                for ctx in ctxs {
                    let mut c = ctx.lock().await;
                    let mine = c.account.as_ref().is_some_and(|(a, _)| a == account);
                    if let Some(s) = c
                        .character
                        .as_mut()
                        .filter(|s| mine && s.name.eq_ignore_ascii_case(&summary.name))
                    {
                        *s = summary.clone();
                    }
                }
            }
            GameEvent::Players { .. } => {}
        }
    }
}

/// Closes `conn` after [`CLOSE_GRACE`], once its last frames are out.
fn close_later(conn: QuicConn, reason: &'static str) {
    tokio::spawn(async move {
        tokio::time::sleep(CLOSE_GRACE).await;
        conn.close(0, reason);
    });
}

/// The kind an unasked `SessionTaken` answers: the login that took the
/// account over.
fn msg_kind_login() -> u8 {
    AccountRequest::Login {
        account: String::new(),
        password: String::new(),
    }
    .kind()
}
