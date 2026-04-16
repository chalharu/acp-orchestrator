(() => {
	const SESSION_ROUTE_PATTERN = /^\/app\/sessions\/([^/]+)$/;
	const SESSION_ID_PATTERN =
		/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
	const elements = {
		backendOrigin: document.getElementById("backend-origin"),
		connectionStatus: document.getElementById("connection-status"),
		sessionStatus: document.getElementById("session-status"),
		routeSummary: document.getElementById("route-summary"),
		errorBanner: document.getElementById("error-banner"),
		transcript: document.getElementById("transcript"),
		pendingPanel: document.getElementById("pending-permissions-panel"),
		pendingPermissions: document.getElementById("pending-permissions"),
		composerForm: document.getElementById("composer-form"),
		composerInput: document.getElementById("composer-input"),
		composerStatus: document.getElementById("composer-status"),
		composerSubmit: document.getElementById("composer-submit"),
	};
	const apiBasePath = normalizeApiBasePath(readMetaContent("acp-api-base"));
	const csrfToken = readMetaContent("acp-csrf-token");
	const origin = globalThis.location.origin;
	const state = {
		sessionId: null,
		sessionStatus: null,
		connectionLabel: "bootstrapping",
		latestSequence: 0,
		pendingPermissions: [],
		messageIds: new Set(),
		eventSource: null,
		routeRevision: 0,
		creatingSession: null,
		sending: false,
	};

	elements.backendOrigin.textContent = origin;
	updateSessionRouteSummary();
	renderEmptyTranscript();
	updateConnectionStatus("bootstrapping");
	updateSessionStatus("new");

	if (!apiBasePath) {
		showFatalBootstrapError(
			"The app shell is missing a valid same-origin API base path.",
		);
		return;
	}
	if (!csrfToken) {
		showFatalBootstrapError("The app shell is missing a CSRF token.");
		return;
	}

	updateComposerState();

	elements.composerForm.addEventListener("submit", (event) => {
		event.preventDefault();
		void handlePromptSubmission();
	});

	elements.composerInput.addEventListener("input", () => {
		updateComposerState();
	});

	elements.composerInput.addEventListener("keydown", (event) => {
		if (event.key === "Enter" && !event.shiftKey) {
			event.preventDefault();
			void handlePromptSubmission();
		}
	});

	globalThis.addEventListener("popstate", () => {
		void bootstrapFromLocation();
	});

	void bootstrapFromLocation();

	function readMetaContent(name) {
		const element = document.querySelector(`meta[name="${name}"]`);
		return element?.getAttribute("content") ?? "";
	}

	function normalizeApiBasePath(value) {
		const trimmed = typeof value === "string" ? value.trim() : "";
		if (
			trimmed.length === 0 ||
			!trimmed.startsWith("/") ||
			trimmed.startsWith("//")
		) {
			return null;
		}
		return trimmed === "/" ? "" : trimmed.replace(/\/+$/, "");
	}

	function normalizeSessionId(value) {
		if (typeof value !== "string") {
			return null;
		}
		const normalized = value.trim().toLowerCase();
		return SESSION_ID_PATTERN.test(normalized) ? normalized : null;
	}

	function requireSessionId(value, context) {
		const sessionId = normalizeSessionId(value);
		if (!sessionId) {
			throw new Error(
				`The backend returned an invalid session id while ${context}.`,
			);
		}
		return sessionId;
	}

	function buildApiUrl(pathname) {
		return new URL(`${apiBasePath}${pathname}`, origin).toString();
	}

	function buildSessionApiUrl(sessionId, suffix = "") {
		const normalizedSessionId = requireSessionId(
			sessionId,
			"building a session request",
		);
		return buildApiUrl(`/sessions/${normalizedSessionId}${suffix}`);
	}

	function buildSessionRoutePath(sessionId) {
		return `/app/sessions/${requireSessionId(sessionId, "building a session route")}`;
	}

	function readSessionEnvelope(payload, context) {
		const session = payload?.session;
		if (!session) {
			throw new Error(
				`The backend returned an invalid session payload while ${context}.`,
			);
		}
		return {
			...session,
			id: requireSessionId(session.id, context),
		};
	}

	function errorMessage(error) {
		return error instanceof Error ? error.message : String(error);
	}

	function parseSessionIdFromLocation() {
		const match = SESSION_ROUTE_PATTERN.exec(globalThis.location.pathname);
		if (!match) {
			return null;
		}
		try {
			return normalizeSessionId(decodeURIComponent(match[1]));
		} catch (error) {
			console.warn("Discarding an invalid session route.", error);
			return null;
		}
	}

	async function bootstrapFromLocation() {
		const routeRevision = claimRouteRevision();
		const sessionId = parseSessionIdFromLocation();
		if (!sessionId) {
			resetForNewChat();
			return;
		}

		await attachExistingSession(sessionId, routeRevision);
	}

	function resetForNewChat() {
		closeEventSource();
		state.sessionId = null;
		state.sessionStatus = null;
		state.latestSequence = 0;
		state.pendingPermissions = [];
		state.messageIds = new Set();
		clearBanner();
		renderTranscriptEntries([]);
		renderPendingPermissions();
		updateConnectionStatus("ready");
		updateSessionStatus("new");
		updateSessionRouteSummary();
		updateComposerState();
	}

	async function attachExistingSession(sessionId, routeRevision) {
		const normalizedSessionId = requireSessionId(
			sessionId,
			"loading a session route",
		);

		closeEventSource();
		clearBanner();
		state.sessionId = normalizedSessionId;
		state.sessionStatus = "loading";
		state.pendingPermissions = [];
		state.messageIds = new Set();
		state.latestSequence = 0;
		renderTranscriptEntries([]);
		renderPendingPermissions();
		updateConnectionStatus("loading history");
		updateSessionStatus("loading");
		updateSessionRouteSummary();
		updateComposerState();

		try {
			const [snapshotResponse, historyResponse] = await Promise.all([
				fetchJson(buildSessionApiUrl(normalizedSessionId)),
				fetchJson(buildSessionApiUrl(normalizedSessionId, "/history")),
			]);
			if (!routeRevisionIsCurrent(routeRevision, normalizedSessionId)) {
				return;
			}

			const session = readSessionEnvelope(
				snapshotResponse,
				"loading a session",
			);
			if (session.id !== normalizedSessionId) {
				throw new Error(
					"The backend returned a snapshot for a different session.",
				);
			}

			state.sessionId = session.id;
			state.sessionStatus = session.status;
			state.latestSequence = Math.max(
				state.latestSequence,
				session.latest_sequence ?? 0,
			);
			updateSessionStatus(session.status);
			updateSessionRouteSummary();
			renderMessages(historyResponse.messages ?? []);
			renderMessages(session.messages ?? []);
			setPendingPermissions(session.pending_permissions ?? []);
			updateComposerState();
			if (session.status === "closed") {
				updateConnectionStatus("closed");
				appendTranscriptEntry(
					"status",
					"status",
					"Session is closed. Transcript is read-only.",
				);
				return;
			}

			connectEventStream(routeRevision);
		} catch (error) {
			if (!routeRevisionIsCurrent(routeRevision, normalizedSessionId)) {
				return;
			}
			state.sessionStatus = "error";
			updateConnectionStatus("error");
			updateSessionStatus("error");
			showBanner(errorMessage(error));
			appendTranscriptEntry(
				"status",
				"status",
				'Failed to load this session. Use "Start a fresh chat" to recover.',
			);
			updateComposerState();
		}
	}

	async function handlePromptSubmission() {
		const text = elements.composerInput.value.trim();
		if (!text || state.sending || elements.composerInput.disabled) {
			return;
		}

		state.sending = true;
		updateComposerState();
		clearBanner();

		try {
			let sessionId = state.sessionId;
			if (!sessionId) {
				sessionId = await createSessionForFirstPrompt();
				if (!sessionId || state.sessionId !== sessionId) {
					return;
				}
			}

			await postJson(buildSessionApiUrl(sessionId, "/messages"), { text });
			elements.composerInput.value = "";
			updateComposerState();
		} catch (error) {
			const message = errorMessage(error);
			showBanner(message);
			appendTranscriptEntry("status", "status", `Send failed: ${message}`);
		} finally {
			state.sending = false;
			updateComposerState();
		}
	}

	async function createSessionForFirstPrompt() {
		if (state.creatingSession) {
			return state.creatingSession;
		}

		const routeRevision = state.routeRevision;
		state.creatingSession = (async () => {
			updateConnectionStatus("creating session");
			const response = await postJson(buildApiUrl("/sessions"), {});
			const session = readSessionEnvelope(response, "creating a session");
			if (!routeRevisionIsCurrent(routeRevision, null)) {
				void closeStaleSession(session.id);
				return null;
			}

			state.sessionId = session.id;
			state.sessionStatus = session.status;
			state.latestSequence = session.latest_sequence ?? 0;
			state.messageIds = new Set();
			renderTranscriptEntries([]);
			renderMessages(session.messages ?? []);
			setPendingPermissions(session.pending_permissions ?? []);
			updateSessionStatus(session.status);
			const nextPath = buildSessionRoutePath(session.id);
			globalThis.history.pushState({}, "", nextPath);
			const nextRouteRevision = claimRouteRevision();
			updateSessionRouteSummary();
			connectEventStream(nextRouteRevision);
			return session.id;
		})();

		try {
			return await state.creatingSession;
		} finally {
			state.creatingSession = null;
		}
	}

	function connectEventStream(routeRevision) {
		const sessionId = parseSessionIdFromLocation();
		if (!sessionId) {
			handleProtocolError(
				"The current route does not contain a valid session id.",
			);
			return;
		}

		closeEventSource();
		updateConnectionStatus("connecting");
		updateSessionRouteSummary();
		const source = new EventSource(buildSessionApiUrl(sessionId, "/events"));
		state.eventSource = source;

		source.onopen = () => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			updateConnectionStatus("live");
		};

		source.onerror = () => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			updateConnectionStatus("reconnecting");
			showBanner("Live updates disconnected. The browser will keep retrying.");
		};

		source.addEventListener("session.snapshot", (event) => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			const payload = parseEventPayload(event);
			if (!payload?.session) {
				return;
			}

			const snapshotSessionId = normalizeSessionId(payload.session.id);
			if (!snapshotSessionId || snapshotSessionId !== sessionId) {
				handleProtocolError(
					"Received a session snapshot for an unexpected session.",
				);
				return;
			}

			state.sessionId = snapshotSessionId;
			state.sessionStatus = payload.session.status;
			state.latestSequence = Math.max(
				state.latestSequence,
				payload.session.latest_sequence ?? 0,
			);
			updateConnectionStatus("live");
			updateSessionStatus(payload.session.status);
			updateSessionRouteSummary();
			renderMessages(payload.session.messages ?? []);
			setPendingPermissions(payload.session.pending_permissions ?? []);
			updateComposerState();
			clearBanner();
		});

		source.addEventListener("conversation.message", (event) => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			const payload = parseEventPayload(event);
			if (!payload?.message) {
				return;
			}
			state.latestSequence = Math.max(
				state.latestSequence,
				payload.sequence ?? 0,
			);
			renderMessages([payload.message]);
		});

		source.addEventListener("tool.permission.requested", (event) => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			const payload = parseEventPayload(event);
			if (!payload?.request) {
				return;
			}
			state.latestSequence = Math.max(
				state.latestSequence,
				payload.sequence ?? 0,
			);
			setPendingPermissions(
				state.pendingPermissions
					.filter(
						(request) => request.request_id !== payload.request.request_id,
					)
					.concat([payload.request]),
			);
			appendTranscriptEntry(
				"status",
				"permission",
				`[${payload.request.request_id}] ${payload.request.summary}`,
			);
		});

		source.addEventListener("status", (event) => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			const payload = parseEventPayload(event);
			if (!payload || typeof payload.message !== "string") {
				return;
			}
			state.latestSequence = Math.max(
				state.latestSequence,
				payload.sequence ?? 0,
			);
			appendTranscriptEntry("status", "status", payload.message);
		});

		source.addEventListener("session.closed", (event) => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			const payload = parseEventPayload(event);
			if (!payload) {
				return;
			}
			state.latestSequence = Math.max(
				state.latestSequence,
				payload.sequence ?? 0,
			);
			state.sessionStatus = "closed";
			closeEventSource();
			setPendingPermissions([]);
			updateConnectionStatus("closed");
			updateSessionStatus("closed");
			updateSessionRouteSummary();
			updateComposerState();
			appendTranscriptEntry(
				"status",
				"status",
				payload.reason ?? "Session closed.",
			);
		});
	}

	function showFatalBootstrapError(message) {
		state.sessionStatus = "error";
		updateConnectionStatus("error");
		updateSessionStatus("error");
		showBanner(message);
		appendTranscriptEntry("status", "status", message);
		elements.composerInput.disabled = true;
		elements.composerSubmit.disabled = true;
		elements.composerStatus.textContent = message;
	}

	function handleProtocolError(message) {
		closeEventSource();
		state.sessionStatus = "error";
		updateConnectionStatus("error");
		updateSessionStatus("error");
		updateComposerState();
		showBanner(message);
		appendTranscriptEntry("status", "status", message);
	}

	function closeEventSource() {
		if (state.eventSource) {
			state.eventSource.close();
			state.eventSource = null;
		}
	}

	function claimRouteRevision() {
		state.routeRevision += 1;
		return state.routeRevision;
	}

	function routeRevisionIsCurrent(routeRevision, sessionId) {
		if (state.routeRevision !== routeRevision) {
			return false;
		}
		return parseSessionIdFromLocation() === sessionId;
	}

	function discardStaleEventSource(source, routeRevision, sessionId) {
		if (routeRevisionIsCurrent(routeRevision, sessionId)) {
			return false;
		}
		source.close();
		if (state.eventSource === source) {
			state.eventSource = null;
		}
		return true;
	}

	async function fetchJson(url, options) {
		const response = await fetch(url, {
			credentials: "same-origin",
			...(options ?? {}),
		});
		if (!response.ok) {
			throw new Error(await readErrorMessage(response));
		}
		return response.json();
	}

	async function postJson(url, payload) {
		return fetchJson(url, {
			method: "POST",
			headers: {
				"content-type": "application/json",
				"x-csrf-token": csrfToken,
			},
			body: JSON.stringify(payload),
		});
	}

	async function closeStaleSession(sessionId) {
		try {
			await postJson(buildSessionApiUrl(sessionId, "/close"), {});
		} catch (error) {
			console.warn("Failed to close a stale browser session.", error);
		}
	}

	async function readErrorMessage(response) {
		try {
			const payload = await response.json();
			if (typeof payload?.error === "string" && payload.error.length > 0) {
				return payload.error;
			}
		} catch (error) {
			console.warn("Failed to decode a backend error payload.", error);
		}
		return `Request failed with ${response.status} ${response.statusText}`;
	}

	function parseEventPayload(event) {
		try {
			return JSON.parse(event.data);
		} catch (error) {
			console.warn("Failed to decode a live event payload.", error);
			showBanner("Received an unreadable live event from the backend.");
			return null;
		}
	}

	function renderMessages(messages) {
		messages.forEach((message) => {
			if (!message?.id || state.messageIds.has(message.id)) {
				return;
			}
			state.messageIds.add(message.id);
			const role = message.role === "assistant" ? "assistant" : "user";
			appendTranscriptEntry(role, role, message.text, message.created_at);
		});
	}

	function setPendingPermissions(requests) {
		state.pendingPermissions = requests.slice();
		renderPendingPermissions();
	}

	function renderPendingPermissions() {
		const requests = state.pendingPermissions;
		elements.pendingPermissions.textContent = "";
		elements.pendingPanel.hidden = requests.length === 0;
		requests.forEach((request) => {
			const item = document.createElement("li");
			item.textContent = `[${request.request_id}] ${request.summary}`;
			elements.pendingPermissions.appendChild(item);
		});
	}

	function renderTranscriptEntries(entries) {
		elements.transcript.textContent = "";
		entries.forEach((entry) => {
			elements.transcript.appendChild(entry);
		});
		renderEmptyTranscript();
	}

	function appendTranscriptEntry(kind, label, text, timestamp) {
		const item = document.createElement("li");
		item.className = `transcript-entry transcript-entry--${kind}`;

		const meta = document.createElement("div");
		meta.className = "transcript-entry__meta";

		const labelElement = document.createElement("strong");
		labelElement.textContent = label;
		meta.appendChild(labelElement);

		const timestampElement = document.createElement("span");
		timestampElement.textContent = formatTimestamp(timestamp);
		meta.appendChild(timestampElement);

		const body = document.createElement("p");
		body.className = "transcript-entry__body";
		body.textContent = text;

		item.appendChild(meta);
		item.appendChild(body);

		const emptyState = elements.transcript.querySelector(
			".transcript-entry--empty",
		);
		if (emptyState) {
			emptyState.remove();
		}

		elements.transcript.appendChild(item);
		item.scrollIntoView({ block: "end" });
	}

	function renderEmptyTranscript() {
		if (elements.transcript.children.length > 0) {
			return;
		}

		const emptyState = document.createElement("li");
		emptyState.className = "transcript-entry transcript-entry--empty";

		const body = document.createElement("p");
		body.className = "transcript-entry__body";
		body.textContent = state.sessionId
			? "Waiting for transcript entries from this session."
			: "Start with a prompt. The first send creates a browser-owned session.";

		emptyState.appendChild(body);
		elements.transcript.appendChild(emptyState);
	}

	function updateConnectionStatus(label) {
		state.connectionLabel = label;
		elements.connectionStatus.textContent = label;
	}

	function updateSessionStatus(label) {
		elements.sessionStatus.textContent = label;
	}

	function updateSessionRouteSummary() {
		if (state.sessionId) {
			elements.routeSummary.textContent = `Attached to ${state.sessionId} at ${globalThis.location.pathname}.`;
			return;
		}
		elements.routeSummary.textContent =
			"Send the first prompt to create a session and move to /app/sessions/{id}.";
	}

	function updateComposerState() {
		const isUnavailable =
			state.sessionStatus === "closed" ||
			state.sessionStatus === "loading" ||
			state.sessionStatus === "error";
		const disabled = state.sending || isUnavailable;
		elements.composerInput.disabled = disabled;
		elements.composerSubmit.disabled =
			disabled || elements.composerInput.value.trim().length === 0;

		if (state.sessionStatus === "closed") {
			elements.composerStatus.textContent =
				"This session is closed. Start a fresh chat to continue.";
			return;
		}
		if (state.sessionStatus === "loading") {
			elements.composerStatus.textContent = "Loading session transcript...";
			return;
		}
		if (state.sessionStatus === "error") {
			elements.composerStatus.textContent =
				"This route could not be attached. Start a fresh chat to continue.";
			return;
		}
		if (state.sending) {
			elements.composerStatus.textContent = state.sessionId
				? "Sending prompt..."
				: "Creating session...";
			return;
		}
		if (!state.sessionId) {
			elements.composerStatus.textContent =
				"Ready. The first send creates a session with same-origin cookie auth.";
			return;
		}
		elements.composerStatus.textContent =
			"Connected over SSE. New messages and status updates will appear below.";
	}

	function showBanner(message) {
		elements.errorBanner.hidden = false;
		elements.errorBanner.textContent = message;
	}

	function clearBanner() {
		elements.errorBanner.hidden = true;
		elements.errorBanner.textContent = "";
	}

	function formatTimestamp(timestamp) {
		if (!timestamp) {
			return new Date().toLocaleTimeString([], {
				hour: "2-digit",
				minute: "2-digit",
				second: "2-digit",
			});
		}

		const value = new Date(timestamp);
		if (Number.isNaN(value.getTime())) {
			return "";
		}

		return value.toLocaleTimeString([], {
			hour: "2-digit",
			minute: "2-digit",
			second: "2-digit",
		});
	}
})();
