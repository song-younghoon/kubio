# Documentation rules

Kubio keeps configuration documentation under this directory.

- docs/proposals/ contains unresolved proposals for upcoming minor versions.
- docs/specs/ contains the agreed specifications.
- Each proposal and specification is a plain-text file named vX.Y.txt.
- The first proposal and specification use the filename v0.1.txt.
- Version changes are managed at the minor-version level. Patch releases do not
  change the specification and do not create a new specification file.
- Proposal and specification files contain only incremental changes introduced
  by that minor version. Read earlier versions as the base specification.
- When a proposal is fully agreed, move it from docs/proposals/ to
  docs/specs/ with the same filename.
- docs/proposals/ must contain only proposals that are still under discussion.
- All documentation files are written in English.
