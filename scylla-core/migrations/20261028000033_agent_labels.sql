-- The labels of the agents. The admins give labels to an App, a pipeline asks labels, and a job
-- keeps the labels of its pipeline at its creation. Only an agent with each label of a job,
-- from the admins or from its host, gets the job. No label: any agent, as before.
ALTER TABLE apps ADD COLUMN labels text[] NOT NULL DEFAULT '{}';

ALTER TABLE pipelines ADD COLUMN agent_labels text[] NOT NULL DEFAULT '{}';

ALTER TABLE jobs ADD COLUMN agent_labels text[] NOT NULL DEFAULT '{}';
