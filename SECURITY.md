# Security

Please do not open a public issue for a security problem.

Report it privately instead, either through GitHub ("Report a vulnerability" on the Security tab of this repository) or by email to hey@qoop.si. Include the version or commit, what you found, and the steps or input that show it.

agentparse only reads transcript files and never writes to them, calls the network or reads credentials, so the main risk is a crafted transcript that makes the parser crash, hang or use unbounded memory. Reports of that kind are welcome.
