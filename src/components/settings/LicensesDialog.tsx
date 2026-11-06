import { useEffect, useState } from "react";
import { Modal } from "../Modal";

interface Texts {
  license: string;
  notice: string;
  thirdParty: string;
}

/** The notices file is Markdown; shown as plain text, without its code fences and bold marks. */
const plain = (md: string) =>
  md
    .replace(/^```[a-z]*\n/gm, "")
    .replace(/\*\*(.+?)\*\*/g, "$1")
    .trim();

/** DistroClaw's own license and the licenses of the third-party software it ships. The texts
 *  are about 1 MB, so they are loaded only when this opens. */
export function LicensesDialog({ onClose }: { onClose: () => void }) {
  const [texts, setTexts] = useState<Texts | null>(null);

  useEffect(() => {
    let alive = true;
    Promise.all([
      import("../../../LICENSE?raw"),
      import("../../../NOTICE?raw"),
      import("../../../THIRD_PARTY_NOTICES.md?raw"),
    ]).then(([license, notice, thirdParty]) => {
      if (alive) setTexts({ license: license.default, notice: notice.default, thirdParty: plain(thirdParty.default) });
    });
    return () => {
      alive = false;
    };
  }, []);

  return (
    <Modal title="Open-source licenses" wide onClose={onClose}>
      {!texts ? (
        <p className="muted small">Loading…</p>
      ) : (
        <>
          <section className="field">
            <label>DistroClaw</label>
            <pre className="license-text">{texts.notice.trim()}</pre>
            <pre className="license-text">{texts.license.trim()}</pre>
          </section>
          <section className="field">
            <label>Third-party software</label>
            <pre className="license-text tall">{texts.thirdParty}</pre>
          </section>
        </>
      )}
    </Modal>
  );
}
