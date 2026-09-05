type Page = "dashboard" | "rules" | "diagnostics" | "settings" | "about";

function Bone({ className = "" }: { className?: string }) {
  return <div className={`animate-pulse rounded-lg bg-ink/10 ${className}`} />;
}

function CardBone({
  className = "",
  children,
}: {
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <div
      className={`rounded-2xl border border-ink/10 bg-surface p-3.5 ${className}`}
    >
      {children}
    </div>
  );
}

function HeaderBones() {
  return (
    <div className="flex items-end justify-between gap-4">
      <div className="space-y-2">
        <Bone className="h-3 w-16" />
        <Bone className="h-7 w-64" />
        <Bone className="h-3 w-80 max-w-full" />
      </div>
      <Bone className="h-12 w-36 rounded-2xl" />
    </div>
  );
}

function TableBones({ rows = 4 }: { rows?: number }) {
  return (
    <CardBone>
      <Bone className="h-4 w-40" />
      <div className="mt-3 space-y-2">
        {Array.from({ length: rows }, (_, index) => (
          <Bone key={index} className="h-8 w-full" />
        ))}
      </div>
    </CardBone>
  );
}

function TileRowBones({ count }: { count: number }) {
  return (
    <div
      className="grid gap-3"
      style={{
        gridTemplateColumns: `repeat(auto-fit, minmax(10rem, 1fr))`,
      }}
    >
      {Array.from({ length: count }, (_, index) => (
        <CardBone key={index}>
          <Bone className="h-4 w-20" />
          <Bone className="mt-3 h-5 w-28" />
          <Bone className="mt-2 h-3 w-full" />
        </CardBone>
      ))}
    </div>
  );
}

function DashboardSkeleton() {
  return (
    <>
      <HeaderBones />
      <TileRowBones count={3} />
      <TileRowBones count={5} />
      <div className="grid gap-3 lg:grid-cols-2">
        <CardBone>
          <Bone className="h-5 w-28" />
          <Bone className="mt-2 h-3 w-44" />
          <Bone className="mt-3 h-9 w-full" />
        </CardBone>
        <CardBone>
          <Bone className="h-5 w-28" />
          <Bone className="mt-2 h-3 w-44" />
          <Bone className="mt-3 h-9 w-full" />
        </CardBone>
      </div>
    </>
  );
}

function RulesSkeleton() {
  return (
    <>
      <HeaderBones />
      <CardBone>
        <div className="flex gap-2">
          <Bone className="h-10 flex-1" />
          <Bone className="h-10 w-28 rounded-xl" />
        </div>
      </CardBone>
      <div className="grid gap-3 lg:grid-cols-2">
        <CardBone>
          <Bone className="h-5 w-24" />
          <Bone className="mt-3 h-8 w-full" />
          <Bone className="mt-2 h-8 w-full" />
        </CardBone>
        <CardBone>
          <Bone className="h-5 w-24" />
          <Bone className="mt-3 h-8 w-full" />
          <Bone className="mt-2 h-8 w-full" />
        </CardBone>
      </div>
      <TableBones rows={5} />
    </>
  );
}

function DiagnosticsSkeleton() {
  return (
    <>
      <HeaderBones />
      <TableBones rows={4} />
      <CardBone>
        <div className="flex gap-2">
          <Bone className="h-10 flex-1" />
          <Bone className="h-10 w-32 rounded-xl" />
        </div>
      </CardBone>
      <div className="grid gap-3 xl:grid-cols-2">
        <TableBones rows={3} />
        <TableBones rows={3} />
      </div>
      <TileRowBones count={3} />
    </>
  );
}

function SettingsSkeleton() {
  return (
    <>
      <HeaderBones />
      <div className="flex gap-2">
        <Bone className="h-9 w-28 rounded-xl" />
        <Bone className="h-9 w-28 rounded-xl" />
      </div>
      <CardBone>
        <div className="grid gap-3 sm:grid-cols-2">
          {Array.from({ length: 6 }, (_, index) => (
            <div key={index} className="space-y-1.5">
              <Bone className="h-3 w-24" />
              <Bone className="h-10 w-full" />
            </div>
          ))}
        </div>
      </CardBone>
    </>
  );
}

function AboutSkeleton() {
  return (
    <>
      <HeaderBones />
      <CardBone>
        <Bone className="h-5 w-40" />
        <Bone className="mt-3 h-3 w-full" />
        <Bone className="mt-2 h-3 w-2/3" />
        <Bone className="mt-4 h-10 w-44 rounded-xl" />
      </CardBone>
    </>
  );
}

/**
 * Structured per-page loading skeleton mirroring each page's real layout,
 * shown while bootstrap data is still on its way.
 */
export function PageSkeleton({ page }: { page: Page }) {
  return (
    <section
      aria-busy="true"
      aria-label="Loading"
      data-testid="page-skeleton"
      className="flex flex-col gap-3 pb-2"
    >
      {page === "dashboard" ? <DashboardSkeleton /> : null}
      {page === "rules" ? <RulesSkeleton /> : null}
      {page === "diagnostics" ? <DiagnosticsSkeleton /> : null}
      {page === "settings" ? <SettingsSkeleton /> : null}
      {page === "about" ? <AboutSkeleton /> : null}
    </section>
  );
}
