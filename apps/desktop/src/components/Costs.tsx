import type { UiState } from "../types";
import { money } from "../machine";

export function Costs({ costs }: { costs: UiState["costs"] }) {
  return (
    <footer className="costs" aria-label="Costs">
      <div className="cost">
        <span className="caption" title="Agent37 counts days in UTC">today</span>
        <span className="cost-value">{money(costs.today)}</span>
      </div>
      <div className="cost">
        <span className="caption">this month</span>
        <span className="cost-value">{money(costs.month)}</span>
      </div>
      <div
        className={`cost${costs.balance !== null && costs.balance < 2 ? " is-warn" : ""}`}
        title="Left in your Agent37 wallet, which every machine draws on"
      >
        <span className="caption">balance</span>
        <span className="cost-value">{costs.balance === null ? "–" : money(costs.balance)}</span>
      </div>
    </footer>
  );
}
