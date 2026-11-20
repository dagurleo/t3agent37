// What a machine costs: Agent37's rates as the README has them (November 2026). Asleep, its disk
// alone; awake, its size's VM rate by the minute. t3agent37 itself is free.
import { SpyIcon } from "./panel/components/SpyIcon";
import { seedOf } from "./panel/machine";

/** Agent37's VM sizes, as t3agent37 offers them. */
const SIZES = [
  { id: "small", spec: "2 vCPU · 4 GB", hourly: "$0.015" },
  { id: "default", spec: "4 vCPU · 8 GB", hourly: "$0.030" },
  { id: "large", spec: "4 vCPU · 16 GB", hourly: "$0.049" },
  { id: "xlarge", spec: "4 vCPU · 32 GB", hourly: "$0.088" },
  { id: "huge", spec: "8 vCPU · 64 GB", hourly: "$0.175" },
];

export function Pricing() {
  return (
    <section className="pricing" id="pricing">
      <p className="eyebrow">Pricing, roughly</p>
      <h2>Asleep, a machine bills its disk. Awake, the minutes it runs.</h2>
      <p className="pricing-lede">
        Agent37 meters by the minute and draws on your prepaid Agent37 wallet. t3agent37 is free and open source: you pay
        Agent37 for the machines, nothing else.
      </p>

      <div className="pricing-grid">
        <div className="price-card">
          <div className="price-head">
            <SpyIcon state="asleep" seed={seedOf("falcon")} />
            <span className="price-tag is-asleep">zzz</span>
          </div>
          <p className="price-name">Asleep</p>
          <p className="price-big">
            $1.80<span>/month</span>
          </p>
          <p className="price-note">About that for its 20 GB disk alone, however long it sleeps. A message from T3 Code wakes it in seconds.</p>
        </div>

        <div className="price-card price-awake">
          <div className="price-head">
            <SpyIcon state="working" seed={seedOf("raven")} />
            <span className="price-tag">working</span>
          </div>
          <p className="price-name">Awake, by size</p>
          <table className="price-table">
            <thead>
              <tr>
                <th scope="col">size</th>
                <th scope="col">VM</th>
                <th scope="col">per hour awake</th>
              </tr>
            </thead>
            <tbody>
              {SIZES.map((s) => (
                <tr key={s.id}>
                  <th scope="row">{s.id}</th>
                  <td>{s.spec}</td>
                  <td>{s.hourly}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      <p className="pricing-example">
        For example, a default machine that works two hours a day, 22 days a month: 44 h × $0.030 + $1.80 for the disk ≈
        $3.12 a month. Rates are Agent37's and may change; check{" "}
        <a href="https://www.agent37.com">agent37.com</a> for current ones.
      </p>
    </section>
  );
}
