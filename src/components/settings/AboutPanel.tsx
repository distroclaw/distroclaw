import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { ext } from "../../ext";
import { Logo } from "../Icons";
import { LicensesDialog } from "./LicensesDialog";

const { EditionInfo } = ext;

export function AboutPanel() {
  const [version, setVersion] = useState<string | null>(null);
  const [licenses, setLicenses] = useState(false);

  useEffect(() => {
    let alive = true;
    getVersion()
      .then((v) => alive && setVersion(v))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  return (
    <>
      <section className="field">
        <div className="label-row">
          <Logo size={28} />
          <b>DistroClaw</b>
        </div>
        <p className="field-help">Private AI that runs entirely on this computer.</p>
      </section>
      <section className="field about">
        <span>
          <b>{ext.editionName}</b>
          {version && ` · Version ${version}`}
        </span>
        {EditionInfo && (
          <span>
            <EditionInfo />
          </span>
        )}
      </section>
      <section className="field">
        <div className="label-row">
          <button className="btn" onClick={() => setLicenses(true)}>
            Open-source licenses
          </button>
        </div>
      </section>
      {licenses && <LicensesDialog onClose={() => setLicenses(false)} />}
    </>
  );
}
