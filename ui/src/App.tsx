import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  Hash,
  Inbox,
  MessagesSquare,
  MessageSquareText,
  Slack,
  X,
} from "lucide-react";

import { Badge, BadgeIcon } from "#components/ds/badge/badge.tsx";
import { Button, ButtonLeadIcon } from "#components/ds/button/button.tsx";
import {
  Typography,
  TypographyH2,
  TypographyP,
} from "#components/ds/typography/typography.tsx";
import {
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from "#components/ui/card.tsx";
import { Textarea } from "#components/ui/textarea.tsx";

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

function Section({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <TypographyP size="xs" weight="medium" variant="subtle">
        {label}
      </TypographyP>
      {children}
      {hint && (
        <TypographyP size="xs" variant="hint">
          {hint}
        </TypographyP>
      )}
    </div>
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
    <Card className="mx-auto w-full max-w-2xl">
      <CardHeader>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <CardTitle>
            <TypographyH2 size="lg" weight="semibold">
              Message approval request
            </TypographyH2>
          </CardTitle>
          <Badge variant="purple" shape="pill">
            {incoming.agent}
          </Badge>
        </div>
        <TypographyP size="xs" variant="muted">
          Request {request.id} · received{" "}
          {new Date(request.received_at * 1000).toLocaleTimeString()}
        </TypographyP>
      </CardHeader>
      <CardContent className="space-y-5">
        <Section label="Proposed message (exact text)">
          <Typography
            tag="div"
            size="sm"
            className="whitespace-pre-wrap break-words rounded-md border border-default bg-subtle px-3 py-2 font-mono"
          >
            {incoming.message}
          </Typography>
        </Section>

        <Section
          label="Destination (Slack)"
          hint="Where the message would be sent — this is not the agent conversation."
        >
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="blue">
              <BadgeIcon icon={Hash} />
              {incoming.destination}
            </Badge>
            {incoming.thread_ts && (
              <Badge variant="neutral">thread {incoming.thread_ts}</Badge>
            )}
          </div>
        </Section>

        <Section label="Reason">
          <TypographyP size="sm">{incoming.reason}</TypographyP>
        </Section>

        <Section
          label="Originating agent conversation"
          hint="Context only — not a Slack thread."
        >
          <div className="space-y-1 rounded-md border border-default bg-card-subtle p-3">
            <div className="flex items-center gap-2">
              <MessagesSquare className="size-4 shrink-0 text-icon-subtle" />
              <TypographyP size="sm" weight="semibold">
                {incoming.conversation.title}
              </TypographyP>
            </div>
            <TypographyP
              size="xs"
              variant="subtle"
              className="break-all font-mono"
            >
              {incoming.conversation.id_or_link}
            </TypographyP>
            <TypographyP size="sm" variant="muted">
              {incoming.conversation.summary}
            </TypographyP>
          </div>
        </Section>
      </CardContent>
      <CardFooter className="flex-col items-stretch gap-3">
        {feedbackMode && (
          <div className="space-y-2">
            <Textarea
              value={feedback}
              onChange={(e) => setFeedback(e.target.value)}
              placeholder="Tell the requesting agent what to change…"
              className="min-h-[80px]"
            />
            <Button
              variant="secondary"
              size="sm"
              isDisabled={!feedback.trim() || deciding}
              onClick={() =>
                onDecide({ type: "feedback", feedback: feedback.trim() })
              }
            >
              <ButtonLeadIcon icon={MessageSquareText} />
              Submit feedback
            </Button>
          </div>
        )}
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Button
            variant="soft"
            isDisabled={deciding}
            onClick={() => setFeedbackMode((v) => !v)}
          >
            <ButtonLeadIcon icon={MessageSquareText} />
            Give feedback
          </Button>
          <Button
            variant="destructive"
            isDisabled={deciding}
            onClick={() => onDecide({ type: "deny" })}
          >
            <ButtonLeadIcon icon={X} />
            Deny
          </Button>
          <Button
            variant="primary"
            isDisabled={deciding}
            onClick={() => onDecide({ type: "approve" })}
          >
            <ButtonLeadIcon icon={Check} />
            Approve
          </Button>
        </div>
      </CardFooter>
    </Card>
  );
}

function EmptyState() {
  return (
    <Card className="mx-auto w-full max-w-2xl">
      <CardContent className="flex flex-col items-center gap-3 py-16 text-center">
        <Inbox className="size-8 text-icon-muted" />
        <TypographyP size="lg" weight="medium">
          Waiting for agent requests
        </TypographyP>
        <TypographyP size="sm" variant="muted">
          When an agent asks to send a Slack message, it appears here for your
          approval.
        </TypographyP>
      </CardContent>
    </Card>
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
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
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
    <div className="flex min-h-screen flex-col bg-default text-default">
      <header className="flex items-center justify-between gap-2 border-b border-default px-6 py-4">
        <div className="flex items-center gap-2">
          <Slack className="size-5 text-icon-default" />
          <TypographyH2 size="lg" weight="semibold">
            Slack Approval Gateway
          </TypographyH2>
        </div>
        {queue.length > 0 && (
          <Badge variant="orange" shape="pill">
            {queue.length} waiting
          </Badge>
        )}
      </header>

      <main className="flex-1 overflow-y-auto p-6">
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
      </main>

      <footer className="space-y-1 border-t border-default px-6 py-3">
        <TypographyP size="xs" variant="hint">
          Closing this window hides it and never approves a message; the
          gateway keeps running (menu-bar icon to reopen or quit). If the
          gateway quits, pending requests fail and the agent is told nothing
          was sent.
        </TypographyP>
        {cooldowns.length > 0 && (
          <TypographyP size="xs" variant="warning">
            Deny cooldowns active:{" "}
            {cooldowns
              .map(
                ([destination, , remaining]) =>
                  `${destination} (${Math.ceil(remaining / 60)} min left)`,
              )
              .join(", ")}
          </TypographyP>
        )}
      </footer>
    </div>
  );
}
