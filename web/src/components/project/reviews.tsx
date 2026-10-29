// A project's review agents: whether an agent reviews each new commit of its pull requests, on
// whose Claude subscription and how, and the reviews they ran, newest first. The project's owners
// turn them on (on their own subscription), take them over and say how they review; the person
// they run for may turn them off; people who start sessions ask for a review of a pull request.

import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  ArrowRight,
  ArrowUpRight,
  Bug,
  CheckCheck,
  ChevronDown,
  CircleCheck,
  Crosshair,
  Settings2,
} from "lucide-react";
import { Fragment, type ReactNode, useEffect, useId, useRef, useState } from "react";
import { BotMark, ReviewBots } from "@/components/project/review-bots";
import { ReviewRepos } from "@/components/project/review-repos";
import { ReviewStateIcon, RISK_TONE, SummaryAlert } from "@/components/review";
import {
  Avatar,
  Banner,
  Button,
  buttonClass,
  Dialog,
  EffortIcon,
  Empty,
  Field,
  Input,
  Logo,
  Meta,
  ReviewIcon,
  Select,
  type SelectOption,
  Skeleton,
  Switch,
  Textarea,
  toast,
} from "@/components/ui";
import { AppAvatar, someNames, useReviewApp } from "@/components/workspace/review-app";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { agoShort, duration, stamp } from "@/lib/format";
import {
  ALIASES,
  chosenModel,
  EFFORTS,
  effortLabel,
  type ModelChoice,
  modelChoice,
  modelLabel,
} from "@/lib/models";
import { keys, queryClient, useMe } from "@/lib/queries";
import { accountOf, postedAsGithubApp, postsAllAs, postsAs } from "@/lib/review-app";
import { botView, sameBots } from "@/lib/review-bots";
import {
  canAskForReview,
  canManageReviews,
  commitUrl,
  earlierFindings,
  pullRef,
  RISKS,
  repoList,
  repoName,
  reviewLook,
  reviewOutcome,
  reviewRepos,
  sameRepos,
  shortSha,
} from "@/lib/reviews";

export function ReviewsTab({ project }: { project: G.Project }) {
  const agents = useQuery({
    queryKey: keys.reviewAgents(project.id),
    queryFn: () => api.reviewAgents(project.id),
  });
  const list = useInfiniteQuery({
    queryKey: keys.reviews(project.id),
    queryFn: ({ pageParam }) => api.reviews(project.id, { before: pageParam, limit: 30 }),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.next_before ?? undefined,
  });
  const [asking, setAsking] = useState(false);
  const items = list.data?.pages.flatMap((p) => p.reviews) ?? [];
  const a = agents.data;
  if (agents.isError) return <Banner tone="error">{agents.error.message}</Banner>;
  if (!a || list.isPending)
    return (
      <div className="space-y-4">
        <Skeleton className="h-24" />
        <Skeleton className="h-12" />
        <Skeleton className="h-12" />
      </div>
    );
  // Never on, and no reviews to show: what they are, and how to turn them on.
  if (!a.enabled && items.length === 0) return <Intro project={project} agents={a} />;
  const ask = a.enabled && canAskForReview(project.role);
  return (
    <div className="flex flex-col gap-10">
      <Status project={project} agents={a} onAsk={ask ? () => setAsking(true) : undefined} />
      <section aria-labelledby="reviews-heading">
        <h2 id="reviews-heading" className="mb-2 text-[14px] font-semibold text-ink">
          Reviews
        </h2>
        {items.length === 0 ? (
          <Empty title="No reviews yet" icon={<ReviewIcon kind="comment" className="size-7" />}>
            {a.repos.length > 0 &&
              `The next commit pushed to a pull request in ${repoList(a.repos)} starts one. Its check shows on the commit, and its findings on the pull request.`}
          </Empty>
        ) : (
          <ul className="border-t border-line">
            {items.map((r) => (
              <ReviewRow key={r.session_id} r={r} />
            ))}
          </ul>
        )}
        {list.isError && <p className="mt-4 text-[13px] text-danger">{list.error.message}</p>}
        {list.hasNextPage && (
          <div className="mt-4">
            <Button
              size="sm"
              variant="ghost"
              loading={list.isFetchingNextPage}
              onClick={() => void list.fetchNextPage()}
            >
              Older
            </Button>
          </div>
        )}
      </section>
      <AskForReview project={project} agents={a} open={asking} onClose={() => setAsking(false)} />
    </div>
  );
}

/** Changes a project's review agents, and shows them as they are now. */
function useUpdate(project: G.Project, done?: (a: G.ReviewAgents) => void) {
  return useMutation({
    mutationFn: (b: G.UpdateReviewAgentsRequest) => api.updateReviewAgents(project.id, b),
    onSuccess: (a) => {
      const before = queryClient.getQueryData<G.ReviewAgents>(keys.reviewAgents(project.id));
      queryClient.setQueryData(keys.reviewAgents(project.id), a);
      // The apps that opened pull requests lately are those of the repositories they review: the
      // settings ask again when they next open (they close as they save).
      if (before && !sameRepos(before.repos.map(repoName), a.repos.map(repoName)))
        void queryClient.invalidateQueries({ queryKey: keys.reviewBots(project.id), refetchType: "none" });
      done?.(a);
    },
  });
}

// ───────────────────────────── before they are on ─────────────────────────────

const POINTS = [
  {
    icon: <Crosshair />,
    text: "It reviews what the pull request changes and what that affects, not the code around it.",
  },
  {
    icon: <Bug />,
    text: "It points out bugs, security problems and breaks of the repository's rules, such as its AGENTS.md, and leaves out nitpicks.",
  },
  {
    icon: <CheckCheck />,
    text: "When a new commit fixes what it found, it marks its finding fixed and resolves the thread.",
  },
  {
    icon: <CircleCheck />,
    text: "Its check passes when there's nothing to point out, and stays neutral when there is: acting on it is up to you.",
  },
];

/** What review agents do, for a project that never had them on, and how to turn them on. */
function Intro({ project, agents }: { project: G.Project; agents: G.ReviewAgents }) {
  const owner = canManageReviews(project.role);
  const on = useUpdate(project, () =>
    toast("Review agents are on: the next commit of each pull request gets a review"),
  );
  const repos = agents.repos.length > 0 ? repoList(agents.repos) : "this project's repositories";
  // The app the reviews would post as, where one posts them all: of those in accounts the workspace
  // works in, and none where GitHub didn't say.
  const reviewApp = useReviewApp(project.workspace?.id).data;
  const known = reviewApp ? agents.repos.filter((repo) => accountOf(reviewApp, repo.owner)) : [];
  const all = reviewApp?.configured && known.length > 0 ? postsAllAs(reviewApp, known) : undefined;
  const poster = all === "review" ? reviewApp?.app : all === "github" ? reviewApp?.github_app : undefined;
  return (
    <div className="grid animate-rise items-center gap-x-12 gap-y-10 py-2 md:grid-cols-[minmax(0,1fr)_320px]">
      <div>
        <h2 className="font-display text-[28px] leading-[1.15] font-medium tracking-[-0.018em] text-ink">
          A review of every new commit
        </h2>
        <p className="mt-3 max-w-xl text-[15px] leading-relaxed text-ink-3">
          When a pull request in {repos} opens or gets a new commit, a review agent reads it and posts what it
          finds there, in one review, with a check on the commit. <Waiting bots={agents.bots} here />
        </p>
        <ul className="mt-6 flex max-w-xl flex-col gap-3">
          {POINTS.map((p) => (
            <li key={p.text} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
              <span aria-hidden="true" className="mt-[3px] shrink-0 text-ink-3 [&_svg]:size-4">
                {p.icon}
              </span>
              {p.text}
            </li>
          ))}
        </ul>
        {agents.problem && (
          <Banner tone="info" className="mt-6 max-w-xl">
            {agents.problem}
          </Banner>
        )}
        {on.error && <OnError error={on.error} />}
        {owner ? (
          <div className="mt-8 flex flex-wrap items-center gap-x-4 gap-y-3">
            <Button
              variant="primary"
              size="lg"
              loading={on.isPending}
              onClick={() => on.mutate({ enabled: true })}
            >
              Turn on review agents
            </Button>
            <span className="text-[12.5px] leading-snug text-ink-4">
              They run on your Claude subscription.
              <br className="max-sm:hidden" /> Another owner can take them over.
            </span>
          </div>
        ) : (
          <p className="mt-8 text-[13px] text-ink-4">
            The project's owners turn them on, on their Claude subscription.
          </p>
        )}
      </div>
      <Preview app={poster} />
    </div>
  );
}

/** Why turning them on failed, with the way out when it is a missing Claude connection. */
function OnError({ error }: { error: Error }) {
  const claude = error instanceof ApiFailure && error.field === "claude";
  return (
    <Banner tone="error" className="mt-6 max-w-xl">
      {error.message}
      {claude && (
        <>
          {" "}
          <Link
            to="/settings"
            search={{ linked: undefined, error: undefined }}
            className="underline underline-offset-2"
          >
            Connect Claude
          </Link>
        </>
      )}
    </Banner>
  );
}

/**
 * Whose pull requests wait for someone to ask for their review: those of people who can't write to
 * the repository, and of the apps other than those that start reviews by themselves (`bots`).
 */
function Waiting({ bots, here }: { bots: readonly string[]; here?: boolean }) {
  const someone = here ? "someone here" : "someone";
  if (bots.length === 0)
    return (
      <>
        Pull requests by apps, and by people who can't write to the repository, wait until {someone} asks for
        their review.
      </>
    );
  return (
    <>
      Pull requests by apps other than <AppNames bots={bots} />, and by people who can't write to the
      repository, wait until {someone} asks for their review.
    </>
  );
}

/** "Dependabot and Renovate", each with its logo; "Dependabot, Renovate and 2 more" past three. */
function AppNames({ bots }: { bots: readonly string[] }) {
  const shown = bots.length > 3 ? bots.slice(0, 2) : bots;
  const parts: ReactNode[] = shown.map((login) => (
    <span key={login} className="inline-flex items-baseline gap-1 whitespace-nowrap text-ink-2">
      <span className="relative top-[2px] self-start">
        <BotMark login={login} size={15} />
      </span>
      {botView(login).name}
    </span>
  ));
  if (bots.length > shown.length) parts.push(`${bots.length - shown.length} more`);
  return (
    <>
      {parts.map((p, i) => (
        <Fragment key={typeof p === "string" ? p : i}>
          {i > 0 && (i === parts.length - 1 ? " and " : ", ")}
          {p}
        </Fragment>
      ))}
    </>
  );
}

/**
 * What a review looks like on GitHub: a finding on the line it's about, and the check on the commit;
 * by `app`, the GitHub App it posts as, where that's known.
 */
function Preview({ app }: { app?: G.GithubAppRef }) {
  return (
    <div aria-hidden="true" className="relative select-none max-md:max-w-sm">
      <div className="overflow-hidden rounded-xl border border-line bg-surface shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)]">
        <div className="flex items-center gap-2 border-b border-line px-3.5 py-2.5 text-[12px]">
          {app ? (
            <AppAvatar app={app} size={18} />
          ) : (
            <Logo withWord={false} className="[&_svg]:size-[18px]" />
          )}
          <span className="font-semibold text-ink">{app?.slug ?? "fctl"}</span>
          <span className="text-ink-4">reviewed</span>
        </div>
        <div className="border-b border-line bg-sunken/60 px-3.5 py-2 font-mono text-[11px] text-ink-3">
          src/sync/upload.rs
        </div>
        <div className="font-mono text-[11px] leading-[1.7] whitespace-pre">
          <div className="flex bg-checks-passed/10">
            <span className="w-9 shrink-0 pr-2 text-right text-ink-4">41</span>
            <span className="text-checks-passed">+</span>
            <span className="pl-1.5 text-ink-2">{"loop {"}</span>
          </div>
          <div className="flex bg-checks-passed/10">
            <span className="w-9 shrink-0 pr-2 text-right text-ink-4">42</span>
            <span className="text-checks-passed">+</span>
            <span className="overflow-hidden pl-1.5 text-ellipsis text-ink-2">
              {"    if send(&chunk).is_ok() { break }"}
            </span>
          </div>
        </div>
        <div className="border-t border-line px-3.5 pt-3 pb-3.5">
          <p className="text-[13px] font-semibold text-ink">Retries never stop when the server is down</p>
          <p className="mt-1.5 text-[12px] leading-relaxed text-ink-3">
            <span className="font-semibold text-ink-2">High severity.</span>{" "}
            <code className="font-mono text-[11px]">upload</code> tries again until a send works, so a server
            that stays down keeps the worker busy for good.
          </p>
        </div>
      </div>
      <div className="mt-3 flex items-center gap-2.5 rounded-xl border border-line bg-surface px-3.5 py-2.5 text-[12px] shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)]">
        <ReviewStateIcon look="commented" className="size-3.5" />
        <span className="font-semibold text-ink">fctl review</span>
        <span className="truncate text-ink-3">Found 1 issue</span>
      </div>
    </div>
  );
}

// ───────────────────────────── on, or off with reviews ─────────────────────────────

/**
 * Whether they are on, which repositories they review, on whose subscription and with what, and
 * why none runs; with their settings a click away for the people who change them.
 */
function Status({
  project,
  agents,
  onAsk,
}: {
  project: G.Project;
  agents: G.ReviewAgents;
  onAsk?: () => void;
}) {
  const { data: me } = useMe();
  const owner = canManageReviews(project.role);
  const mine = !!me && agents.operator?.id === me.user.id;
  const [open, setOpen] = useState(false);
  const [off, setOff] = useState(false);
  const panel = useId();
  const on = useUpdate(project, () => toast("Review agents are on"));
  const operator = agents.operator;
  const model = agents.model ? modelLabel(agents.model) : undefined;
  const effort = agents.effort ? effortLabel(agents.effort).toLowerCase() : undefined;
  return (
    <div>
      <div className="overflow-hidden rounded-xl border border-line bg-surface/80">
        <div className="flex flex-wrap items-start gap-x-6 gap-y-4 px-5 py-4">
          <div className="min-w-0 flex-1 basis-80">
            <p className="flex items-center gap-2 text-[14px] font-semibold text-ink">
              <span
                aria-hidden="true"
                className={cn("size-2 rounded-full", agents.enabled ? "bg-checks-passed" : "bg-ink-4")}
              />
              {agents.enabled ? "Review agents are on" : "Review agents are off"}
            </p>
            <p className="mt-1.5 text-[13.5px] leading-relaxed text-ink-3">
              {!agents.enabled ? (
                "Pull requests get no new reviews. The reviews they had stay below."
              ) : agents.repos.length === 0 ? (
                // Why they review none shows below.
                operator ? (
                  <>
                    They run on <Whose operator={operator} mine={mine} /> Claude subscription
                    <How model={model} effort={effort} />.
                  </>
                ) : (
                  "They review no pull requests."
                )
              ) : (
                <>
                  They review each new commit of the pull requests in{" "}
                  <span className="text-ink-2">{repoList(agents.repos)}</span>
                  {operator && (
                    <>
                      , on <Whose operator={operator} mine={mine} /> Claude subscription
                    </>
                  )}
                  <How model={model} effort={effort} />.{" "}
                  {agents.skipped_repos.length > 0 && (
                    <>
                      They skip <span className="text-ink-2">{repoList(agents.skipped_repos)}</span>.{" "}
                    </>
                  )}
                  {agents.description_summary &&
                    "Before each review, they sum the pull request up at the end of its description. "}
                  <Waiting bots={agents.bots} />
                </>
              )}
            </p>
          </div>
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            {onAsk && (
              <Button size="md" onClick={onAsk}>
                Review a pull request
              </Button>
            )}
            {!agents.enabled && owner && (
              <Button variant="primary" loading={on.isPending} onClick={() => on.mutate({ enabled: true })}>
                Turn on
              </Button>
            )}
            {agents.enabled && owner && (
              <Button
                variant="ghost"
                icon={<Settings2 className="size-4" />}
                trailing={
                  <ChevronDown
                    className={cn("size-3.5 transition-transform duration-200", open && "rotate-180")}
                  />
                }
                aria-expanded={open}
                aria-controls={panel}
                onClick={() => setOpen(!open)}
              >
                Settings
              </Button>
            )}
            {agents.enabled && !owner && mine && (
              <Button variant="ghost" onClick={() => setOff(true)}>
                Turn off
              </Button>
            )}
          </div>
        </div>
        {agents.enabled && <PostedAs project={project} agents={agents} />}
        {open && owner && (
          <div id={panel} className="animate-rise border-t border-line px-5 py-5">
            <SettingsForm
              project={project}
              agents={agents}
              mine={mine}
              onDone={() => setOpen(false)}
              onTurnOff={() => setOff(true)}
            />
          </div>
        )}
      </div>
      {on.error && <OnError error={on.error} />}
      {agents.problem && (agents.enabled || owner) && (
        <Banner tone={agents.enabled ? "warn" : "info"} className="mt-3">
          {agents.problem}
        </Banner>
      )}
      <TurnOff
        project={project}
        open={off}
        onClose={() => setOff(false)}
        onDone={() => {
          setOff(false);
          setOpen(false);
        }}
      />
    </div>
  );
}

/**
 * Which GitHub App the reviews post as: the review app where it's installed (fctl-review), apart
 * from what agents write as the GitHub App (fctl-sh), which posts them elsewhere; with the way to
 * install it where it isn't, for the workspace's admins, or where they do it for the others.
 */
function PostedAs({ project, agents }: { project: G.Project; agents: G.ReviewAgents }) {
  const q = useReviewApp(project.workspace?.id);
  const r = q.data;
  if (!r?.configured || !r.app || !r.github_app) return null;
  const { app, github_app: ours } = r;
  // Not those of accounts the workspace no longer works in, which get no reviews.
  const repos = agents.repos.filter((repo) => accountOf(r, repo.owner));
  if (repos.length === 0) return null;
  // Where GitHub didn't say for some, nothing is said of all of them.
  const unsure = repos.some((repo) => postsAs(r, repo) === undefined);
  const all = postsAllAs(r, repos);
  const elsewhere = postedAsGithubApp(r, repos);
  if (all !== "review" && elsewhere.repos.length === 0) return null;
  const [only] = elsewhere.accounts;
  const install = elsewhere.accounts.length === 1 ? only?.install_url : undefined;
  // Why the others post as the GitHub App: it isn't installed on their accounts, or doesn't reach them.
  const uninstalled =
    elsewhere.accounts.length > 0 && elsewhere.accounts.every((a) => a.state === "not_installed");
  const accounts = someNames(elsewhere.accounts.map((a) => a.account));
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2.5 border-t border-line px-5 py-3 text-[13px] leading-relaxed text-ink-3">
      <AppAvatar app={all === "github" || unsure ? ours : app} size={18} />
      <p className="min-w-0 flex-1 basis-64">
        {all === "review" ? (
          <>
            On GitHub they post as <span className="font-medium text-ink-2">{app.slug}</span>, apart from what
            your agents write as {ours.slug}.
          </>
        ) : unsure ? (
          <>
            On GitHub, reviews in {repoList(elsewhere.repos)} post as{" "}
            <span className="font-medium text-ink-2">{ours.slug}</span>
            {uninstalled
              ? `: ${app.slug} isn't installed on ${accounts}`
              : `, which ${app.slug} doesn't reach`}
            .
          </>
        ) : all === "github" ? (
          <>
            On GitHub they post as <span className="font-medium text-ink-2">{ours.slug}</span>, as your agents
            do: {app.slug}{" "}
            {uninstalled ? `isn't installed on ${accounts}` : `doesn't reach ${repoList(elsewhere.repos)}`}.
          </>
        ) : (
          <>
            On GitHub they post as <span className="font-medium text-ink-2">{app.slug}</span>, but as{" "}
            {ours.slug} in {repoList(elsewhere.repos)}
            {uninstalled ? `: it isn't installed on ${accounts}` : ", which it doesn't reach"}.
          </>
        )}
      </p>
      {elsewhere.repos.length > 0 &&
        (install && only ? (
          <a href={install} className={buttonClass({ size: "sm" })}>
            <AppAvatar app={app} size={14} />
            {only.state === "partly"
              ? `Add ${only.unreached.length === 1 ? only.unreached[0] : "repositories"}`
              : `Install ${app.slug}`}
          </a>
        ) : (
          project.workspace && (
            <Link
              to="/w/$slug"
              params={{ slug: project.workspace.slug }}
              search={{ tab: "github" }}
              className="inline-flex items-center gap-1 text-[12.5px] text-ink-2 hover:text-ink hover:underline"
            >
              {elsewhere.accounts.some((a) => a.install_url) ? `Install ${app.slug}` : "Where it's installed"}
              <ArrowRight className="size-3.5" />
            </Link>
          )
        ))}
    </div>
  );
}

/** "your" or "Ada's", with their avatar: whose Claude subscription they run on. */
function Whose({ operator, mine }: { operator: G.UserRef; mine: boolean }) {
  return (
    <span className="inline-flex items-baseline gap-1.5 text-ink-2">
      <Avatar user={operator} size={16} />
      <span className="self-baseline">{mine ? "your" : `${operator.name}'s`}</span>
    </span>
  );
}

/** ", with Opus at high effort", where they don't take the project's model and effort level. */
function How({ model, effort }: { model?: string; effort?: string }) {
  if (!model && !effort) return null;
  return (
    <>
      , with <span className="text-ink-2">{model ?? "the project's model"}</span>
      {effort && (
        <>
          {" "}
          at <span className="text-ink-2">{effort}</span> effort
        </>
      )}
    </>
  );
}

/** The models review agents may run on: the project's (`""`), the aliases, or a model ID. */
function modelOptions(project: G.Project): SelectOption<ModelChoice>[] {
  const own = project.settings.model?.trim();
  return [
    { value: "", label: "The project's", description: own ? modelLabel(own) : "Claude's default" },
    ...ALIASES,
    { value: "id", label: "Model ID" },
  ];
}

function effortOptions(project: G.Project): SelectOption[] {
  const own = project.settings.effort?.trim() ?? "";
  return [
    { value: "", label: "The project's", description: effortLabel(own), icon: <EffortIcon level={own} /> },
    ...EFFORTS.map((e) => ({ ...e, icon: <EffortIcon level={e.value} /> })),
  ];
}

/**
 * Whose subscription they run on, the repositories they review, their model, effort level and
 * instructions, and the apps that start reviews: for the project's owners.
 */
function SettingsForm({
  project,
  agents,
  mine,
  onDone,
  onTurnOff,
}: {
  project: G.Project;
  agents: G.ReviewAgents;
  mine: boolean;
  onDone: () => void;
  onTurnOff: () => void;
}) {
  const [model, setModel] = useState<ModelChoice>(modelChoice(agents.model));
  const [modelId, setModelId] = useState(modelChoice(agents.model) === "id" ? (agents.model ?? "") : "");
  const [effort, setEffort] = useState(agents.effort ?? "");
  const [instructions, setInstructions] = useState(agents.instructions ?? "");
  const [bots, setBots] = useState<string[]>(agents.bots);
  const [summary, setSummary] = useState(agents.description_summary);
  const repos = reviewRepos(agents, project.repo);
  const [skipped, setSkipped] = useState<string[]>(agents.skipped_repos.map(repoName));
  // Those the project's agents may still write to, should a repository leave meanwhile.
  const skips = skipped.filter((s) => repos.some((r) => repoName(r).toLowerCase() === s.toLowerCase()));
  const skipsChanged = !sameRepos(skips, agents.skipped_repos.map(repoName));
  const save = useUpdate(project, () => {
    toast("Saved: the next review runs with it");
    onDone();
  });
  const takeOver = useUpdate(project, () => toast("Review agents run on your Claude subscription now"));
  const err = (f: string) =>
    (save.error as { field?: string } | null)?.field === f ? save.error?.message : undefined;
  const changed =
    (chosenModel(model, modelId) ?? "") !== (agents.model ?? "") ||
    effort !== (agents.effort ?? "") ||
    instructions.trim() !== (agents.instructions ?? "").trim() ||
    !sameBots(bots, agents.bots) ||
    summary !== agents.description_summary ||
    skipsChanged;
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (model === "id" && !modelId.trim()) {
          save.reset();
          return;
        }
        save.mutate({
          model: chosenModel(model, modelId) ?? "",
          effort,
          instructions,
          bots,
          description_summary: summary,
          ...(skipsChanged && { skipped_repos: skips }),
        });
      }}
      className="flex flex-col gap-5"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="text-[12.5px] font-medium text-ink-2">Runs on</p>
          {agents.operator && (
            <p className="mt-1.5 flex items-center gap-2 text-[13.5px] text-ink">
              <Avatar user={agents.operator} size={22} />
              {mine ? "Your Claude subscription" : `${agents.operator.name}'s Claude subscription`}
            </p>
          )}
        </div>
        {!mine && (
          <Button
            type="button"
            size="sm"
            loading={takeOver.isPending}
            onClick={() => takeOver.mutate({ run_as_me: true })}
          >
            Run them on mine
          </Button>
        )}
      </div>
      {takeOver.error && <OnError error={takeOver.error} />}
      <ReviewRepos
        repos={repos}
        primary={project.repo}
        skipped={skips}
        onChange={setSkipped}
        error={err("skipped_repos")}
      />
      <div className="grid grid-cols-2 items-start gap-4">
        <Field label="Model" error={err("model")}>
          {(id) => (
            <Select
              id={id}
              value={model}
              onChange={setModel}
              options={modelOptions(project)}
              className="w-full"
            />
          )}
        </Field>
        <Field label="Effort" error={err("effort")}>
          {(id) => (
            <Select
              id={id}
              value={effort}
              onChange={setEffort}
              options={effortOptions(project)}
              className="w-full"
            />
          )}
        </Field>
      </div>
      {model === "id" && (
        <Field label="Model ID" hint="Passed to Claude Code as --model." error={err("model")}>
          {(id) => (
            <Input
              id={id}
              value={modelId}
              onChange={(e) => setModelId(e.target.value)}
              className="font-mono"
              placeholder="claude-opus-5-5"
              autoComplete="off"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          )}
        </Field>
      )}
      <Field
        label="Instructions for reviews"
        hint="Review agents get these after fctl's own review instructions: what matters most in this project, or what to leave alone. The project's instructions for its other agents don't apply."
        error={err("instructions")}
      >
        {(id) => (
          <Textarea
            id={id}
            value={instructions}
            onChange={(e) => setInstructions(e.target.value)}
            minRows={3}
            maxRows={14}
            placeholder="Check that database migrations can be rolled back. Don't comment on the generated files in web/src/gen."
          />
        )}
      </Field>
      <DescriptionSummary on={summary} onChange={setSummary} />
      <ReviewBots project={project} value={bots} saved={agents.bots} onChange={setBots} error={err("bots")} />
      {save.error && !(save.error as { field?: string }).field && (
        <Banner tone="error">{save.error.message}</Banner>
      )}
      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-4">
        <Button type="button" variant="quiet" size="sm" onClick={onTurnOff}>
          Turn off review agents
        </Button>
        <div className="flex gap-2">
          <Button type="button" variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={save.isPending} disabled={!changed}>
            Save
          </Button>
        </div>
      </div>
    </form>
  );
}

/** What the preview of a summary says: of the pull request the finding in the intro's preview is in. */
const EXAMPLE: G.PullRequestSummary = {
  risk: "medium",
  risk_reason:
    "It changes how uploads retry, which every sync goes through, though only for uploads that fail.",
  overview:
    "Makes the upload worker give up after five failed attempts and wait longer between them, so that a server that stays down no longer keeps the worker busy for good.",
};

/**
 * Whether review agents sum each pull request up at the end of its description before they review
 * it, and what that looks like on GitHub, once it's on.
 */
function DescriptionSummary({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  const id = useId();
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <p id={id} className="text-[12.5px] font-medium text-ink-2">
            Summary in the description
          </p>
          <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
            Before reviewing in depth, the agent adds how risky the pull request looks, what it does and what
            deserves a closer look at the end of its description, and rewrites it for each new commit. The
            rest of the description stays as people wrote it.
          </p>
        </div>
        <Switch checked={on} onChange={onChange} label="Sum up pull requests in their descriptions" />
      </div>
      {on && (
        <div aria-hidden="true" className="animate-rise rounded-lg border border-line bg-surface px-4 py-3.5">
          <SummaryAlert summary={EXAMPLE} />
          <p className="mt-2.5 pl-[17.5px] text-[11.5px] text-ink-4">
            fctl review's first look at <span className="font-mono">3f2a1c9</span> · 3 files, +120 −4 · Review
            agent settings
          </p>
        </div>
      )}
    </div>
  );
}

/** Asks before turning them off: reviews under way finish, and only owners turn them on again. */
function TurnOff({
  project,
  open,
  onClose,
  onDone,
}: {
  project: G.Project;
  open: boolean;
  onClose: () => void;
  onDone: () => void;
}) {
  const off = useUpdate(project, () => {
    toast("Review agents are off");
    onDone();
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Turn off review agents?"
      description="Reviews under way finish and post. New commits get no review until one of the project's owners turns them on again."
    >
      <div className="flex flex-col gap-5">
        {off.error && <Banner tone="error">{off.error.message}</Banner>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" loading={off.isPending} onClick={() => off.mutate({ enabled: false })}>
            Turn off
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/** Reviews a pull request's head now, on the subscription the project's review agents run on. */
function AskForReview({
  project,
  agents,
  open,
  onClose,
}: {
  project: G.Project;
  agents: G.ReviewAgents;
  open: boolean;
  onClose: () => void;
}) {
  const [pr, setPr] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const start = useMutation({
    mutationFn: () => api.startReview(project.id, { pull_request: pr }),
    onSuccess: (r) => {
      toast(`Reviewing ${pullRef(r)} at ${shortSha(r.head_sha)}`);
      void queryClient.invalidateQueries({ queryKey: keys.reviews(project.id) });
      setPr("");
      onClose();
    },
  });
  useEffect(() => {
    if (open) {
      start.reset();
      requestAnimationFrame(() => input.current?.focus());
    }
  }, [open, start.reset]);
  const operator = agents.operator?.name;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Review a pull request"
      description={`A review agent reviews its newest commit now${operator ? `, on ${operator}'s Claude subscription` : ""}, in place of a review of it still under way.`}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (pr.trim()) start.mutate();
        }}
        className="flex flex-col gap-5"
      >
        <Field
          label="Pull request"
          hint={
            project.repo
              ? `Its number in ${project.repo.owner}/${project.repo.name}, owner/name#12, or its link.`
              : "owner/name#12, or its link."
          }
          error={start.error?.message}
        >
          {(id) => (
            <Input
              ref={input}
              id={id}
              value={pr}
              onChange={(e) => setPr(e.target.value)}
              placeholder={project.repo ? "12" : "acme/web#12"}
              className="font-mono"
              autoComplete="off"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          )}
        </Field>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={start.isPending} disabled={!pr.trim()}>
            Review
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

// ───────────────────────────── the reviews ─────────────────────────────

const OUTCOME_TONE: Record<ReturnType<typeof reviewLook>, string> = {
  waiting: "text-ink-3",
  reviewing: "text-accent",
  passed: "text-checks-passed",
  commented: "text-blue",
  superseded: "text-ink-4",
  stopped: "text-ink-4",
  failed: "text-danger",
};

/**
 * A review: its pull request and commit, what came of it and how long it took, on whose
 * subscription it ran and when. The row opens its session; its pull request, commit and review
 * link to GitHub.
 */
function ReviewRow({ r }: { r: G.Review }) {
  const look = reviewLook(r);
  const earlier = earlierFindings(r);
  const ended = look === "superseded" || look === "stopped" || look === "failed";
  return (
    <li className="group relative isolate flex items-start gap-3.5 border-b border-line px-3 py-3.5 transition-colors hover:bg-hover/30">
      <ReviewStateIcon look={look} className="mt-[3px] size-4" />
      <div className="min-w-0 flex-1">
        <Link
          to="/s/$id"
          params={{ id: r.session_id }}
          className="block truncate text-[14.5px] text-ink outline-none after:absolute after:inset-0 after:z-[1] after:rounded focus-visible:after:outline-2 focus-visible:after:outline-offset-2 focus-visible:after:outline-accent"
        >
          {r.title || `Pull request #${r.number}`}
          {r.draft && <span className="ml-2 align-[1px] text-[11.5px] text-ink-4">Draft</span>}
        </Link>
        <Meta
          parts={[
            {
              key: "pr",
              node: (
                <a
                  href={r.url}
                  target="_blank"
                  rel="noreferrer"
                  title={`${r.repo.owner}/${r.repo.name}#${r.number} on GitHub`}
                  className="relative z-[2] font-mono text-[12px] text-ink-3 underline-offset-2 hover:text-ink hover:underline"
                >
                  {pullRef(r)}
                </a>
              ),
            },
            {
              key: "commit",
              node: (
                <a
                  href={commitUrl(r)}
                  target="_blank"
                  rel="noreferrer"
                  title="The commit it reviewed, on GitHub"
                  className="relative z-[2] font-mono text-[12px] text-ink-3 underline-offset-2 hover:text-ink hover:underline"
                >
                  {shortSha(r.head_sha)}
                </a>
              ),
            },
            {
              key: "outcome",
              node: <span className={cn("font-medium", OUTCOME_TONE[look])}>{reviewOutcome(r)}</span>,
            },
            ...earlier.map((e) => ({ key: e, node: e })),
            r.pull_request_summary && {
              key: "risk",
              node: (
                <span
                  title={`At first sight: ${r.pull_request_summary.risk_reason}`}
                  className={RISK_TONE[r.pull_request_summary.risk]}
                >
                  {RISKS[r.pull_request_summary.risk].label}
                </span>
              ),
            },
            r.busy_ms > 0 &&
              (look === "passed" || look === "commented") && {
                key: "time",
                node: <span title="How long its agent worked">{duration(r.busy_ms)}</span>,
                className: "max-sm:hidden",
              },
            r.review_url && {
              key: "review",
              node: (
                <a
                  href={r.review_url}
                  target="_blank"
                  rel="noreferrer"
                  className="relative z-[2] inline-flex items-center gap-0.5 text-ink-3 underline-offset-2 hover:text-ink hover:underline"
                >
                  Review on GitHub <ArrowUpRight className="size-3" />
                </a>
              ),
            },
          ]}
        />
        {ended && r.reason && (
          <p className="mt-1 text-[12.5px] leading-relaxed text-ink-4 [overflow-wrap:anywhere]">{r.reason}</p>
        )}
        {r.check_error && (
          <p className="mt-1 text-[12.5px] leading-relaxed text-amber [overflow-wrap:anywhere]">
            {r.check_error}
          </p>
        )}
      </div>
      <span className="mt-0.5 max-sm:hidden" title={`On ${r.operator.name}'s Claude subscription`}>
        <Avatar user={r.operator} size={20} />
      </span>
      <time
        dateTime={new Date(r.created_at).toISOString()}
        title={stamp(r.created_at)}
        className="mt-[3px] w-10 shrink-0 text-right text-[12px] whitespace-nowrap text-ink-4 tabular-nums"
      >
        {agoShort(r.created_at)}
      </time>
      <ArrowRight className="mt-[5px] size-3.5 shrink-0 text-ink-4 opacity-0 transition-all duration-200 group-hover:translate-x-0.5 group-hover:opacity-100 max-sm:hidden" />
    </li>
  );
}
