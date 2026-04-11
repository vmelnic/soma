// Phase 1f proof — plan-following dispatch in the browser.
//
// Takes a shortcut around organic schema induction (which needs
// multi-step episodes the single-skill hello pack doesn't naturally
// produce — same limitation soma-project-multistep called out on
// native) and directly injects a compiled Routine into the routine
// store. Then runs the same goal again and asserts that the session
// controller loads the routine, sets WorkingMemory.active_plan, and
// reports plan_following: true in the final summary.
//
// This is the same approach soma-project-multistep uses: bypass the
// learning pipeline to prove the plan-following DISPATCH machinery
// works, independent of the separate question of whether PrefixSpan
// can produce meaningful schemas from single-skill episodes. Phase 1g
// / phase 2 can tackle organic learning once the dispatch path is
// trusted.

import { test, expect } from "@playwright/test";

const HELLO_ROUTINE = {
  routine_id: "phase1f.hello.routine",
  namespace: "soma.browser.hello",
  origin: "pack_authored",
  match_conditions: [
    {
      condition_type: "goal_fingerprint_match",
      // precondition_matches walks the expression object and asserts
      // every key/value matches the context object. The session
      // controller builds the context as {"goal_fingerprint": <objective>}.
      expression: { goal_fingerprint: "hello marcu" },
      description: "matches the canonical hello marcu goal",
    },
  ],
  compiled_skill_path: ["soma.browser.hello.say_hello"],
  guard_conditions: [],
  expected_cost: 0.001,
  expected_effect: [],
  confidence: 0.95,
};

async function waitForBoot(page) {
  await page.waitForFunction(
    () => {
      const text = document.getElementById("record")?.textContent || "";
      return text.includes('"booted": true');
    },
    { timeout: 15_000 },
  );
}

async function readLastRecord(page) {
  const text = await page.locator("#record").textContent();
  return JSON.parse(text);
}

test.describe("phase 1f — plan-following via injected routine", () => {
  test.beforeEach(async ({ page }) => {
    page.on("console", (msg) => {
      const text = msg.text();
      if (text.startsWith("[soma-next") || text.startsWith("[voice]")) {
        console.log(`    > ${text}`);
      }
    });
    page.on("pageerror", (err) => {
      console.error(`    > page error: ${err.message}`);
    });

    await page.goto("/index.html");
    await waitForBoot(page);
  });

  test("injected routine activates plan-following on next goal run", async ({
    page,
  }) => {
    // 1. Baseline: run the goal without any routines present.
    //    plan_following MUST be false, routine_count MUST be 0.
    await page.fill("#goal-text", "hello marcu");
    await page.click("#btn-run-goal");
    await page.waitForFunction(
      () =>
        document.getElementById("record")?.textContent?.includes('"status"'),
      { timeout: 10_000 },
    );
    const baseline = await readLastRecord(page);
    expect(baseline.status).toBe("completed");
    expect(baseline.plan_following).toBe(false);
    expect(baseline.routine_count).toBe(0);
    expect(baseline.last_skill).toBe("soma.browser.hello.say_hello");

    // 2. Inject a routine that maps the "hello marcu" goal to the
    //    same skill that deliberation picked. Verify the injection
    //    acknowledgement shows our compiled_skill_path.
    const injectResult = await page.evaluate(async (routine) => {
      const mod = await import("./pkg/soma_next.js");
      return mod.soma_inject_routine(JSON.stringify(routine));
    }, HELLO_ROUTINE);
    const injected = JSON.parse(injectResult);
    expect(injected.injected).toBe(true);
    expect(injected.routine_id).toBe("phase1f.hello.routine");
    expect(injected.compiled_skill_path).toEqual([
      "soma.browser.hello.say_hello",
    ]);

    // 3. Run the same goal again. The session controller should find
    //    the routine via RoutineMemoryAdapter::retrieve_matching (which
    //    uses goal_fingerprint), load compiled_skill_path into
    //    active_plan, and flip plan_following to true.
    await page.click("#btn-run-goal");
    // Wait for the #record pane to update to the NEW run — we know it
    // updated when the logged run count in #goal-log increments.
    await page.waitForFunction(() => {
      const log = document.getElementById("goal-log")?.textContent || "";
      return log.includes("#2");
    });
    const followup = await readLastRecord(page);
    expect(followup.status).toBe("completed");
    expect(followup.routine_count).toBe(1);
    expect(followup.plan_following).toBe(true);
    expect(followup.last_skill).toBe("soma.browser.hello.say_hello");

    // 4. Both runs should have produced a visible <h1>hello marcu</h1>.
    const headings = page
      .locator('h1[data-soma="true"]')
      .filter({ hasText: "hello marcu" });
    await expect(headings).toHaveCount(2);

    // Dump the comparison for human-eye debug even on pass.
    console.log(
      `    > baseline: plan_following=${baseline.plan_following} ` +
        `elapsed_ms=${baseline.elapsed_ms} steps=${baseline.steps}`,
    );
    console.log(
      `    > followup: plan_following=${followup.plan_following} ` +
        `elapsed_ms=${followup.elapsed_ms} steps=${followup.steps}`,
    );
  });

  test("injected routine matches only the declared goal fingerprint", async ({
    page,
  }) => {
    // Inject the hello marcu routine and then run a DIFFERENT goal.
    // The routine should NOT match, plan_following should stay false.
    await page.evaluate(async (routine) => {
      const mod = await import("./pkg/soma_next.js");
      mod.soma_inject_routine(JSON.stringify(routine));
    }, HELLO_ROUTINE);

    await page.fill("#goal-text", "different objective text");
    await page.click("#btn-run-goal");
    await page.waitForFunction(
      () =>
        document.getElementById("record")?.textContent?.includes('"status"'),
      { timeout: 10_000 },
    );
    const result = await readLastRecord(page);
    expect(result.status).toBe("completed");
    expect(result.routine_count).toBe(1); // the injected routine is still registered
    expect(result.plan_following).toBe(false); // but did not match this goal
  });
});
