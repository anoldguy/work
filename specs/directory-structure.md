# Directory Structure

How the `work` CLI organizes repositories on the local filesystem.

## Work Directory

All repositories are stored under a single configurable root directory, called the **work directory**. The default is `~/code`.

## Domain-Based Layout

The tool supports multiple GitHub domains (github.com and GitHub Enterprise instances). Each domain is configured independently. One domain may be marked as the **default domain**.

The local path for a repository depends on whether its domain is the default:

| Domain type | Path pattern |
|---|---|
| Default | `{work_dir}/{org}/{repo}` |
| Non-default | `{work_dir}/{domain}/{org}/{repo}` |

The default domain's name is omitted from the path hierarchy, keeping paths short for the most commonly used host.

### Examples

Assuming `~/code` as the work directory and `github.com` as the default domain:

- `github.com/basecamp/bc3` clones to `~/code/basecamp/bc3`
- `github.enterprise.com/corp/internal-app` clones to `~/code/github.enterprise.com/corp/internal-app`

## Path Computation

Given a fully-qualified repo slug (`{domain}/{org}/{repo}`), the tool resolves the local filesystem path by checking whether the slug's domain matches the configured default. The domain segment is included in the path only for non-default domains.

## Auto-Creation

When cloning a repository, any missing parent directories in the computed path are created automatically.

## Repository Detection

A directory is considered a cloned repository if it contains a `.git` subdirectory.
