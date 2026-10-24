import { useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { Hash, MessagesSquare, Slack } from "lucide-react";

import { Button } from "@once-ui-system/core/components/Button";
import { Column } from "@once-ui-system/core/components/Column";
import { Icon } from "@once-ui-system/core/components/Icon";
import { Row } from "@once-ui-system/core/components/Row";
import { StatusIndicator } from "@once-ui-system/core/components/StatusIndicator";
import { Tag } from "@once-ui-system/core/components/Tag";
import { Text } from "@once-ui-system/core/components/Text";
import { Textarea } from "@once-ui-system/core/components/Textarea";

import { AnimatedNumber } from "@/components/motion-primitives/animated-number";
import { Spotlight } from "@/components/motion-primitives/spotlight";
import { TextEffect } from "@/components/motion-primitives/text-effect";

// Mirrors the Rust types in src/core.rs (MessageRequest / IncomingRequest /
// ConversationContext), serialized over Tauri events.
interface ConversationContext {
  title: string;
  id_or_link: string;
  summary: string;
}

interface MessageRequest {
  id: string;
  incoming: {
    message: string;
    destination: string;
    thread_ts: string | null;
    reason: string;
    agent: string;
    conversation: ConversationContext;
  };
  received_at: number;
}

type DecisionPayload =
  | { type: "approve" }
  | { type: "deny" }
  | { type: "feedback"; feedback: string };

type Cooldown = [destination: string, excerpt: string, remainingSecs: number];

// `?demo` seeds a sample request so the popup can be inspected in a plain
// browser (`vite preview`, optionally with `&theme=dark`) without a running
// gateway. It is inert inside the Tauri app, where the URL has no query.
const DEMO = new URLSearchParams(window.location.search).has("demo");

const demoRequest: MessageRequest = {
  id: "demo-0001",
  incoming: {
    message:
      "Heads-up: gateway v0.3 finished deploying — 12/12 checks green. Rolling out to the remaining workspaces now.",
    destination: "#eng-deploys",
    thread_ts: "1729612800.123456",
    reason:
      "The team asked for deploy notifications in this channel; the canary passed and the runbook says to announce.",
    agent: "claude-code",
    conversation: {
      title: "Ship gateway v0.3",
      id_or_link:
        "https://app.superset.sh/workspaces/speakeasy/tasks/deploy-gateway-v03",
      summary:
        "Agent finished the release checklist (build, tests, smoke test, changelog) and asks to announce the deploy in #eng-deploys.",
    },
  },
  received_at: Math.floor(Date.now() / 1000),
};

function Section({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <Column gap="8">
      <Text variant="label-default-xs" onBackground="neutral-medium">
        {label}
      </Text>
      {children}
      {hint && (
        <Text variant="body-default-xs" onBackground="neutral-medium">
          {hint}
        </Text>
      )}
    </Column>
  );
}

function RequestCard({
  request,
  deciding,
  onDecide,
}: {
  request: MessageRequest;
  deciding: boolean;
  onDecide: (decision: DecisionPayload) => void;
}) {
  const [feedbackMode, setFeedbackMode] = useState(false);
  const [feedback, setFeedback] = useState("");
  const { incoming } = request;

  return (
    <Column background="surface" border radius="l" padding="24" gap="20">
      <Spotlight size={260} />
      <Row horizontal="between" vertical="start" gap="16" wrap>
        <Column gap="4">
          <Text variant="display-strong-s" as="h2">
            Message approval request
          </Text>
          <Text variant="body-default-xs" onBackground="neutral-medium">
            Request {request.id} · received{" "}
            {new Date(request.received_at * 1000).toLocaleTimeString()}
          </Text>
        </Column>
        <Tag scheme="brand" prefixIcon="person" label={incoming.agent} />
      </Row>

      <Section label="Proposed message (exact text)">
        <Column background="page" border radius="m" padding="12">
          <Text
            variant="code-default-s"
            style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}
          >
            {incoming.message}
          </Text>
        </Column>
      </Section>

      <Section
        label="Destination (Slack)"
        hint="Where the message would be sent — this is not the agent conversation."
      >
        <Row gap="8" vertical="center" wrap>
          <Row gap="4" vertical="center" onBackground="info-medium">
            <Hash size={14} aria-hidden />
            <Tag scheme="info" label={incoming.destination} />
          </Row>
          {incoming.thread_ts && (
            <Tag
              scheme="neutral"
              size="s"
              label={`thread ${incoming.thread_ts}`}
            />
          )}
        </Row>
      </Section>

      <Section label="Reason">
        <Text variant="body-default-s">{incoming.reason}</Text>
      </Section>

      <Section
        label="Originating agent conversation"
        hint="Context only — not a Slack thread."
      >
        <Column background="page" border radius="m" padding="16" gap="8">
          <Row gap="8" vertical="center" onBackground="neutral-strong">
            <MessagesSquare size={16} aria-hidden />
            <Text variant="body-strong-s">{incoming.conversation.title}</Text>
          </Row>
          <Text
            variant="code-default-xs"
            onBackground="neutral-medium"
            style={{ overflowWrap: "anywhere" }}
          >
            {incoming.conversation.id_or_link}
          </Text>
          <Text variant="body-default-s" onBackground="neutral-medium">
            {incoming.conversation.summary}
          </Text>
        </Column>
      </Section>

      <Column gap="12">
        {feedbackMode && (
          <Column gap="8">
            <Textarea
              id="feedback"
              label="Feedback for the requesting agent"
              placeholder="Tell the agent what to change…"
              value={feedback}
              onChange={(e) => setFeedback(e.target.value)}
              disabled={deciding}
              focusRing
            />
            <Row horizontal="end">
              <Button
                variant="secondary"
                size="s"
                prefixIcon="chat"
                label="Submit feedback"
                disabled={deciding || !feedback.trim()}
                onClick={() =>
                  onDecide({ type: "feedback", feedback: feedback.trim() })
                }
              />
            </Row>
          </Column>
        )}
        <Row horizontal="end" gap="8" wrap>
          <Button
            variant="secondary"
            size="s"
            prefixIcon="chat"
            label="Give feedback"
            disabled={deciding}
            onClick={() => setFeedbackMode((v) => !v)}
          />
          <Button
            variant="danger"
            size="s"
            prefixIcon="close"
            label="Deny"
            disabled={deciding}
            onClick={() => onDecide({ type: "deny" })}
          />
          <Button
            variant="primary"
            size="s"
            prefixIcon="check"
            label="Approve"
            disabled={deciding}
            onClick={() => onDecide({ type: "approve" })}
          />
        </Row>
      </Column>
    </Column>
  );
}

function EmptyState() {
  return (
    <Column background="surface" border radius="l" padding="48" center gap="12">
      <Icon name="inbox" size="l" onBackground="neutral-medium" />
      <Text variant="body-strong-m">Waiting for agent requests</Text>
      <Text
        variant="body-default-s"
        onBackground="neutral-medium"
        align="center"
      >
        When an agent asks to send a Slack message, it appears here for your
        approval.
      </Text>
    </Column>
  );
}

export default function App() {
  const [queue, setQueue] = useState<MessageRequest[]>([]);
  const [cooldowns, setCooldowns] = useState<Cooldown[]>([]);
  const [deciding, setDeciding] = useState(false);
  const current = queue[0];

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      // Register the listener BEFORE telling the backend we are ready, so
      // requests queued in the backend's outbox cannot be flushed into a
      // webview that is not listening yet.
      const off = await listen<MessageRequest>("approval-request", (event) => {
        setQueue((q) => [...q, event.payload]);
      });
      if (cancelled) {
        off();
        return;
      }
      unlisten = off;
      await invoke("ui_ready");
      invoke<Cooldown[]>("get_active_cooldowns")
        .then(setCooldowns)
        .catch(() => {});
    })().catch(() => {
      // Outside Tauri (vite preview) there is no IPC; the demo seed below
      // stands in for live requests.
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!DEMO) return;
    setQueue([demoRequest]);
    setCooldowns([["#eng-deploys", "Heads-up: gateway v0.3…", 7 * 60 + 23]]);
  }, []);

  useEffect(() => {
    document.title =
      queue.length > 0
        ? `Slack Approval Gateway — ${queue.length} pending`
        : "Slack Approval Gateway";
  }, [queue.length]);

  const decide = (decision: DecisionPayload) => {
    if (!current || deciding) return;
    setDeciding(true);
    invoke("submit_decision", { requestId: current.id, decision })
      .catch(() => {})
      .finally(() => {
        setQueue((q) => q.slice(1));
        setDeciding(false);
        // The popup appears per request batch: once the queue has drained,
        // hide the window (hiding never quits the gateway; the next request
        // or the menu-bar icon brings it back).
        if (queue.length <= 1) {
          invoke("hide_window").catch(() => {});
        }
        invoke<Cooldown[]>("get_active_cooldowns")
          .then(setCooldowns)
          .catch(() => {});
      });
  };

  return (
    <Column
      fillWidth
      fillHeight
      background="page"
      onBackground="neutral-strong"
      overflow="hidden"
    >
      <Row
        as="header"
        horizontal="between"
        vertical="center"
        gap="16"
        paddingX="24"
        paddingY="16"
        borderBottom
        wrap
      >
        <Row gap="8" vertical="center">
          <Slack
            size={20}
            aria-hidden
            style={{ color: "var(--brand-on-background-strong)" }}
          />
          <TextEffect
            as="h2"
            per="word"
            preset="fade-in-blur"
            className="font-display font-strong font-m"
          >
            Slack Approval Gateway
          </TextEffect>
        </Row>
        {queue.length > 0 && (
          <Tag scheme="warning">
            <AnimatedNumber value={queue.length} /> waiting
          </Tag>
        )}
      </Row>

      <Column
        as="main"
        flex="1"
        fillWidth
        overflowY="auto"
        horizontal="center"
        padding="24"
      >
        <Column fillWidth maxWidth="l">
          {current ? (
            <RequestCard
              key={current.id}
              request={current}
              deciding={deciding}
              onDecide={decide}
            />
          ) : (
            <EmptyState />
          )}
        </Column>
      </Column>

      <Column as="footer" borderTop paddingX="24" paddingY="12" gap="4">
        <Text variant="body-default-xs" onBackground="neutral-medium">
          Closing this window hides it and never approves a message; the
          gateway keeps running (menu-bar icon to reopen or quit). If the
          gateway quits, pending requests fail and the agent is told nothing
          was sent.
        </Text>
        {cooldowns.length > 0 && (
          <Row gap="8" vertical="center">
            <StatusIndicator size="s" color="warning" />
            <Text variant="body-default-xs" onBackground="warning-medium">
              Deny cooldowns active:{" "}
              {cooldowns
                .map(
                  ([destination, , remaining]) =>
                    `${destination} (${Math.ceil(remaining / 60)} min left)`,
                )
                .join(", ")}
            </Text>
          </Row>
        )}
      </Column>
    </Column>
  );
}
