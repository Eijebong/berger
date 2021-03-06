Berger
======

This is a very small treeherder alternative developped as a pet project to
monitor the divvun CI pipelines. It has a very very small subset of what
treeherder actually does and is tailored to my need although I'm open to
suggestions.

# Deployment

To deploy this, you can run the docker container at
`ghcr.io/eijebong/berger:main`, you'll need to provide it the following
environment variables:

- `TASKCLUSTER_ROOT_URL`: The root URL of your taskcluster instance, example: `https://taskcluster.your.org`
- `AMQP_ADDR`: The address of the rabbitmq used for taskcluster's pulse, example: `amqp://user:password@rabbitmq.taskcluster.your.org`
- `DATABASE_URL`: The URL of the postgresql database berger will use to store its data, example: `postgres://user@password@postgres.taskcluster.your.org/berger`
- `REDIRECT_URL`(optional): If your instance is not deployed as `berger.${TASKCLUSTER_ROOT_URL}`, set this to `berger.url/auth/callback`
- `GITHUB_TRIGGER_HOOK`(optional): The hook to call to trigger jobs for github repositories. Example: `project/github-trigger`. More on that in the `Trigger hook` section.

To use the login feature, you have to add an oauth client definition to your taskcluster instance.

Example:

`registered_clients: [ {"clientId": "berger", "responseType": "code", "scope": ["berger:*", "hooks:trigger-hook:*], "whitelisted": true, "redirectUri": ["https://berger.taskcluster.url/auth/callback"], "maxExpires": "1 year" } ]`

# Scopes

To avoid leaking repositories, berger uses scopes to filter what's displayed.
You should add a `berger:get-repo:org/name` scope to roles that should have
access to those repositories. You can also replace `org/name` by a `*` to allow
all repositories.

# Usage

This will pick up every task that's created on your taskcluster instance,
there's currently no filtering. It'll assume two things about created tasks.

First, the source of the decision task should be github's `${event.compare}`.
Second, you should add a tag with the current ref named `git_ref`.

```
metadata:
  source: ${event.compare}
  ...
tags:
  git_ref: ${event.ref}
  ...
```

This is due to taskcluster not having any way to link a task to a push event
except for the source and custom metadata like tags.
Technically the `git_ref` is not necessary to link to a push but then you
cannot differentiate a branch push and a tag push as the source would show up
as the same. Note that I'm open to any way of simplificating these requirements.

## Trigger hook

Berger includes a way to run jobs from a predefined hook. This is useful to
avoid requiring empty commits to rerun some CI. It works by triggering the hook
specified by `GITHUB_TRIGGER_HOOK` with the following arguments:
  - repo\_org
  - repo\_name
  - branch

You can then craft a hook that runs your decision task for a push for that repo. The hooks should have the following trigger schema:

```
type: object
properties:
  branch:
    type: string
  repo_org:
    type: string
  repo_name:
    type: string
additionalProperties: false
```
