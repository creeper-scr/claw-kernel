"""
Scheduler demo — create cron-based scheduled tasks.
"""

import time

from claw_kernel import KernelClient, SessionConfig


def main():
    with KernelClient() as client:
        # Create a session for the scheduler
        session_id = client.session.create(
            SessionConfig(system_prompt="You are a scheduled assistant.")
        )
        print(f"Session: {session_id}")

        # Schedule a task to run every minute
        task = client.schedule.create(
            session_id=session_id,
            cron="* * * * *",
            prompt="Provide a brief status update.",
            label="minutely-status",
        )
        print(f"Scheduled task created: {task.task_id} (cron: {task.cron})")

        # Schedule a one-shot task
        oneshot = client.schedule.create(
            session_id=session_id,
            cron="once",
            prompt="Say hello once!",
            label="one-shot-hello",
        )
        print(f"One-shot task: {oneshot.task_id}")

        # List scheduled tasks
        tasks = client.schedule.list(session_id)
        print(f"\nScheduled tasks ({len(tasks)}):")
        for t in tasks:
            print(
                f"  - [{t.task_id}] {t.label or '(no label)'} | {t.cron} | {t.status}"
            )

        # Cancel the recurring task
        client.schedule.cancel(task.task_id)
        print(f"\nCancelled task: {task.task_id}")

        # Clean up
        client.session.destroy(session_id)
        print("Done.")


if __name__ == "__main__":
    main()
