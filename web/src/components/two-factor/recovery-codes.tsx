import { Download, Printer } from "lucide-react";
import { createPortal } from "react-dom";
import { Button, CopyButton, Logo } from "@/components/ui";
import { useInstance } from "@/lib/queries";
import { recoveryCodesFile, recoveryCodesText } from "@/lib/two-factor";

/** Recovery codes, shown once: to copy, download or print. Printing prints a page of only them. */
export function RecoveryCodes({ codes, email }: { codes: string[]; email: string }) {
  const { data: instance } = useInstance();
  const name = instance?.name ?? "fctl";
  const download = () => {
    const text = recoveryCodesText(codes, name, email, new Date());
    const url = URL.createObjectURL(new Blob([text], { type: "text/plain;charset=utf-8" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = recoveryCodesFile(name);
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  const print = () => {
    const html = document.documentElement;
    html.classList.add("printing-codes");
    addEventListener("afterprint", () => html.classList.remove("printing-codes"), { once: true });
    window.print();
  };
  return (
    <div>
      <ol className="grid grid-cols-2 gap-x-4 gap-y-2.5 rounded-lg border border-line-strong bg-sunken px-3.5 py-4 font-mono text-[13px] tracking-[0.03em] text-ink sm:gap-x-6 sm:px-5 sm:text-[14px] sm:tracking-[0.04em]">
        {codes.map((c, i) => (
          <li
            key={c}
            className="flex animate-rise items-baseline gap-3"
            style={{ animationDelay: `${i * 28}ms` }}
          >
            <span className="w-4 shrink-0 text-right text-[11px] text-ink-4 tabular-nums">{i + 1}</span>
            <span className="whitespace-nowrap select-all">{c}</span>
          </li>
        ))}
      </ol>
      <div className="mt-2.5 flex flex-wrap items-center gap-1">
        <CopyButton text={codes.join("\n")} label="Copy all" />
        <Button size="sm" variant="ghost" icon={<Download className="size-3.5" />} onClick={download}>
          Download
        </Button>
        <Button size="sm" variant="ghost" icon={<Printer className="size-3.5" />} onClick={print}>
          Print
        </Button>
      </div>
      {createPortal(<PrintSheet codes={codes} email={email} instance={name} />, document.body)}
    </div>
  );
}

/** The page that prints: only the codes, with whose they are and what they do. */
function PrintSheet({ codes, email, instance }: { codes: string[]; email: string; instance: string }) {
  const made = new Date().toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
  return (
    <div className="print-sheet" aria-hidden="true">
      <Logo />
      <h1 className="mt-10 text-[26px] font-semibold tracking-[-0.01em]">Recovery codes</h1>
      <p className="mt-2 text-[14px] leading-relaxed">
        For {email} on {instance}, made on {made}. Each code signs you in once, in place of a code from your
        authenticator app. Getting new codes replaces them all.
      </p>
      <ol className="mt-8 grid grid-cols-2 gap-x-12 gap-y-4 font-mono text-[17px] tracking-[0.06em]">
        {codes.map((c, i) => (
          <li key={c} className="flex items-baseline gap-4">
            <span className="w-5 text-right text-[12px] opacity-50">{i + 1}</span>
            <span>{c}</span>
          </li>
        ))}
      </ol>
      <p className="mt-10 text-[12px] opacity-60">Keep this page somewhere safe, away from your phone.</p>
    </div>
  );
}
