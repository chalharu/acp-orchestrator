(() => {
	const apiBase = readMetaContent("acp-api-base");
	const csrfToken = readMetaContent("acp-csrf-token");
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

	elements.backendOrigin.textContent = window.location.origin;
	updateSessionRouteSummary();
	renderEmptyTranscript();
	updateConnectionStatus("bootstrapping");
	updateSessionStatus("new");
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

	window.addEventListener("popstate", () => {
		void bootstrapFromLocation();
	});

	void bootstrapFromLocation();

	function readMetaContent(name) {
		const element = document.querySelector('meta[name="' + name + '"]');
		return element ? element.getAttribute("content") || "" : "";
	}

	function parseSessionIdFromLocation() {
		const match = window.location.pathname.match(/^\/app\/sessions\/([^/]+)$/);
		if (!match) {
			return null;
		}
		try {
			return decodeURIComponent(match[1]);
		} catch (_error) {
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
		closeEventSource();
		clearBanner();
		state.sessionId = sessionId;
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
			const encodedSessionId = encodeURIComponent(sessionId);
			const [snapshotResponse, historyResponse] = await Promise.all([
				fetchJson(apiBase + "/sessions/" + encodedSessionId),
				fetchJson(apiBase + "/sessions/" + encodedSessionId + "/history"),
			]);
			if (!routeRevisionIsCurrent(routeRevision, sessionId)) {
				return;
			}

			const session = snapshotResponse.session;
			state.sessionId = session.id;
			state.sessionStatus = session.status;
			state.latestSequence = Math.max(
				state.latestSequence,
				session.latest_sequence || 0,
			);
			updateSessionStatus(session.status);
			updateSessionRouteSummary();
			renderMessages(historyResponse.messages || []);
			renderMessages(session.messages || []);
			setPendingPermissions(session.pending_permissions || []);
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

			connectEventStream(session.id, routeRevision);
		} catch (error) {
			if (!routeRevisionIsCurrent(routeRevision, sessionId)) {
				return;
			}
			state.sessionStatus = "error";
			updateConnectionStatus("error");
			updateSessionStatus("error");
			showBanner(error.message);
			appendTranscriptEntry(
				"status",
				"status",
				"Failed to load this session. Use “Start a fresh chat” to recover.",
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

			await postJson(
				apiBase + "/sessions/" + encodeURIComponent(sessionId) + "/messages",
				{ text: text },
			);
			elements.composerInput.value = "";
			updateComposerState();
		} catch (error) {
			showBanner(error.message);
			appendTranscriptEntry(
				"status",
				"status",
				"Send failed: " + error.message,
			);
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
			const response = await postJson(apiBase + "/sessions", {});
			const session = response.session;
			if (!routeRevisionIsCurrent(routeRevision, null)) {
				void closeStaleSession(session.id);
				return null;
			}

			state.sessionId = session.id;
			state.sessionStatus = session.status;
			state.latestSequence = session.latest_sequence || 0;
			state.messageIds = new Set();
			renderTranscriptEntries([]);
			renderMessages(session.messages || []);
			setPendingPermissions(session.pending_permissions || []);
			updateSessionStatus(session.status);
			const nextPath = "/app/sessions/" + encodeURIComponent(session.id);
			window.history.pushState({}, "", nextPath);
			const nextRouteRevision = claimRouteRevision();
			updateSessionRouteSummary();
			connectEventStream(session.id, nextRouteRevision);
			return session.id;
		})();

		try {
			return await state.creatingSession;
		} finally {
			state.creatingSession = null;
		}
	}

	function connectEventStream(sessionId, routeRevision) {
		closeEventSource();
		updateConnectionStatus("connecting");
		updateSessionRouteSummary();
		const source = new EventSource(
			apiBase + "/sessions/" + encodeURIComponent(sessionId) + "/events",
		);
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
			if (!payload || !payload.session) {
				return;
			}

			const session = payload.session;
			state.sessionId = session.id;
			state.sessionStatus = session.status;
			state.latestSequence = Math.max(
				state.latestSequence,
				session.latest_sequence || 0,
			);
			updateConnectionStatus("live");
			updateSessionStatus(session.status);
			updateSessionRouteSummary();
			renderMessages(session.messages || []);
			setPendingPermissions(session.pending_permissions || []);
			updateComposerState();
			clearBanner();
		});

		source.addEventListener("conversation.message", (event) => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			const payload = parseEventPayload(event);
			if (!payload || !payload.message) {
				return;
			}
			state.latestSequence = Math.max(
				state.latestSequence,
				payload.sequence || 0,
			);
			renderMessages([payload.message]);
		});

		source.addEventListener("tool.permission.requested", (event) => {
			if (discardStaleEventSource(source, routeRevision, sessionId)) {
				return;
			}
			const payload = parseEventPayload(event);
			if (!payload || !payload.request) {
				return;
			}
			state.latestSequence = Math.max(
				state.latestSequence,
				payload.sequence || 0,
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
				"[" + payload.request.request_id + "] " + payload.request.summary,
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
				payload.sequence || 0,
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
				payload.sequence || 0,
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
				payload.reason || "Session closed.",
			);
		});
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
		const response = await fetch(
			url,
			Object.assign({ credentials: "same-origin" }, options || {}),
		);
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
			await postJson(
				apiBase + "/sessions/" + encodeURIComponent(sessionId) + "/close",
				{},
			);
		} catch (error) {
			console.warn("Failed to close a stale browser session.", error);
		}
	}

	async function readErrorMessage(response) {
		try {
			const payload = await response.json();
			if (
				payload &&
				typeof payload.error === "string" &&
				payload.error.length > 0
			) {
				return payload.error;
			}
		} catch (_error) {
			// Fall back to status text below.
		}
		return "Request failed with " + response.status + " " + response.statusText;
	}

	function parseEventPayload(event) {
		try {
			return JSON.parse(event.data);
		} catch (_error) {
			showBanner("Received an unreadable live event from the backend.");
			return null;
		}
	}

	function renderMessages(messages) {
		messages.forEach((message) => {
			if (!message || !message.id || state.messageIds.has(message.id)) {
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
			item.textContent = "[" + request.request_id + "] " + request.summary;
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
		item.className = "transcript-entry transcript-entry--" + kind;

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
			elements.routeSummary.textContent =
				"Attached to " +
				state.sessionId +
				" at " +
				window.location.pathname +
				".";
		} else {
			elements.routeSummary.textContent =
				"Send the first prompt to create a session and move to /app/sessions/{id}.";
		}
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
