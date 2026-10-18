import { Download, Printer } from "lucide-react";
import { Button, CopyButton } from "@/components/ui";
import { useInstance } from "@/lib/queries";
import { recoveryCodesFile, recoveryCodesText } from "@/lib/two-factor";

/** Recovery codes, shown once: to copy, download or print. Printing prints only them. */
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
    <div className="print-codes">
      <div className="mb-6 hidden print:block">
        <h1 className="text-[22px] font-semibold">{name} recovery codes</h1>
        <p className="mt-1 text-[14px]">
          For {email}. Each code signs you in once, in place of a code from your authenticator app.
        </p>
      </div>
      <ol className="grid grid-cols-2 gap-x-6 gap-y-2.5 rounded-lg border border-line-strong bg-sunken px-5 py-4 font-mono text-[14px] tracking-[0.04em] text-ink">
        {codes.map((c, i) => (
          <li
            key={c}
            className="flex animate-rise items-baseline gap-3"
            style={{ animationDelay: `${i * 28}ms` }}
          >
            <span className="w-4 shrink-0 text-right text-[11px] text-ink-4 tabular-nums">{i + 1}</span>
            <span className="select-all">{c}</span>
          </li>
        ))}
      </ol>
      <div className="print-hide mt-2.5 flex flex-wrap items-center gap-1">
        <CopyButton text={codes.join("\n")} label="Copy all" />
        <Button size="sm" variant="ghost" icon={<Download className="size-3.5" />} onClick={download}>
          Download
        </Button>
        <Button size="sm" variant="ghost" icon={<Printer className="size-3.5" />} onClick={print}>
          Print
        </Button>
      </div>
    </div>
  );
}
