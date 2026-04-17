import {useEffect, useMemo, useRef, useState} from "react";
import {useMutation, useQueryClient} from "@tanstack/react-query";
import {api, type AcpProfile} from "@/api/client";
import {Button, Input, SelectContent, SelectItem, SelectRoot, SelectTrigger, SelectValue} from "@spacedrive/primitives";
import type {GlobalSettingsSectionProps} from "./types";
import {PERMISSION_OPTIONS} from "./constants";

type WorkerTab = "opencode" | "acp";

function profileArgsToString(args: string[]) {
	return args.join("\n");
}

function parseArgs(args: string) {
	return args
		.split("\n")
		.map((value) => value.trim())
		.filter(Boolean);
}

function cloneProfiles(profiles: AcpProfile[]) {
	return profiles.map((profile) => ({
		...profile,
		args: [...profile.args],
		env: {...profile.env},
	}));
}

export function CodeWorkersSection({settings, isLoading}: GlobalSettingsSectionProps) {
	const queryClient = useQueryClient();
	const [activeTab, setActiveTab] = useState<WorkerTab>("opencode");
	const envKeyCounter = useRef(0);

	const [enabled, setEnabled] = useState(settings?.opencode?.enabled ?? false);
	const [path, setPath] = useState(settings?.opencode?.path ?? "opencode");
	const [maxServers, setMaxServers] = useState(
		settings?.opencode?.max_servers?.toString() ?? "5",
	);
	const [startupTimeout, setStartupTimeout] = useState(
		settings?.opencode?.server_startup_timeout_secs?.toString() ?? "30",
	);
	const [maxRetries, setMaxRetries] = useState(
		settings?.opencode?.max_restart_retries?.toString() ?? "5",
	);
	const [editPerm, setEditPerm] = useState(
		settings?.opencode?.permissions?.edit ?? "allow",
	);
	const [bashPerm, setBashPerm] = useState(
		settings?.opencode?.permissions?.bash ?? "allow",
	);
	const [webfetchPerm, setWebfetchPerm] = useState(
		settings?.opencode?.permissions?.webfetch ?? "allow",
	);

	const [acpEnabled, setAcpEnabled] = useState(settings?.acp?.enabled ?? false);
	const [acpHandshakeTimeout, setAcpHandshakeTimeout] = useState(
		settings?.acp?.handshake_timeout_secs?.toString() ?? "20",
	);
	const [acpStderrBuffer, setAcpStderrBuffer] = useState(
		settings?.acp?.stderr_buffer_bytes?.toString() ?? "16384",
	);
	const [acpProfiles, setAcpProfiles] = useState<AcpProfile[]>(
		cloneProfiles(settings?.acp?.profiles ?? []),
	);
	const [message, setMessage] = useState<{
		text: string;
		type: "success" | "error";
	} | null>(null);

	useEffect(() => {
		if (!settings) return;
		setEnabled(settings.opencode.enabled);
		setPath(settings.opencode.path);
		setMaxServers(settings.opencode.max_servers.toString());
		setStartupTimeout(settings.opencode.server_startup_timeout_secs.toString());
		setMaxRetries(settings.opencode.max_restart_retries.toString());
		setEditPerm(settings.opencode.permissions.edit);
		setBashPerm(settings.opencode.permissions.bash);
		setWebfetchPerm(settings.opencode.permissions.webfetch);

		setAcpEnabled(settings.acp.enabled);
		setAcpHandshakeTimeout(settings.acp.handshake_timeout_secs.toString());
		setAcpStderrBuffer(settings.acp.stderr_buffer_bytes.toString());
		setAcpProfiles(cloneProfiles(settings.acp.profiles));
	}, [settings]);

	const updateMutation = useMutation({
		mutationFn: api.updateGlobalSettings,
		onSuccess: (result) => {
			if (result.success) {
				setMessage({text: result.message, type: "success"});
				queryClient.invalidateQueries({queryKey: ["global-settings"]});
			} else {
				setMessage({text: result.message, type: "error"});
			}
		},
		onError: (error) => {
			setMessage({text: `Failed: ${error.message}`, type: "error"});
		},
	});

	const acpProfilesValid = useMemo(
		() =>
			acpProfiles.every(
				(profile) => profile.id.trim() && profile.command.trim(),
			),
		[acpProfiles],
	);

	const saveOpenCode = () => {
		const servers = parseInt(maxServers, 10);
		const timeout = parseInt(startupTimeout, 10);
		const retries = parseInt(maxRetries, 10);
		if (isNaN(servers) || servers < 1) {
			setMessage({text: "Max servers must be at least 1", type: "error"});
			return;
		}
		if (isNaN(timeout) || timeout < 1) {
			setMessage({text: "Startup timeout must be at least 1", type: "error"});
			return;
		}
		if (isNaN(retries) || retries < 0) {
			setMessage({text: "Max retries cannot be negative", type: "error"});
			return;
		}

		updateMutation.mutate({
			opencode: {
				enabled,
				path: path.trim() || "opencode",
				max_servers: servers,
				server_startup_timeout_secs: timeout,
				max_restart_retries: retries,
				permissions: {
					edit: editPerm,
					bash: bashPerm,
					webfetch: webfetchPerm,
				},
			},
		});
	};

	const saveAcp = () => {
		const timeout = parseInt(acpHandshakeTimeout, 10);
		const stderrBytes = parseInt(acpStderrBuffer, 10);
		if (isNaN(timeout) || timeout < 1) {
			setMessage({text: "Handshake timeout must be at least 1", type: "error"});
			return;
		}
		if (isNaN(stderrBytes) || stderrBytes < 1024) {
			setMessage({text: "stderr buffer must be at least 1024 bytes", type: "error"});
			return;
		}
		if (!acpProfilesValid) {
			setMessage({text: "Every ACP profile needs an id and command", type: "error"});
			return;
		}

		updateMutation.mutate({
			acp: {
				enabled: acpEnabled,
				handshake_timeout_secs: timeout,
				stderr_buffer_bytes: stderrBytes,
				profiles: acpProfiles,
			},
		});
	};

	const addProfile = () => {
		setAcpProfiles((profiles) => [
			...profiles,
			{
				id: "",
				display_name: "",
				command: "",
				args: [],
				env: {},
			},
		]);
	};

	const updateProfile = (
		index: number,
		update: Partial<AcpProfile>,
	) => {
		setAcpProfiles((profiles) =>
			profiles.map((profile, profileIndex) =>
				profileIndex === index ? {...profile, ...update} : profile,
			),
		);
	};

	const updateProfileEnv = (
		index: number,
		key: string,
		value: string,
	) => {
		setAcpProfiles((profiles) =>
			profiles.map((profile, profileIndex) => {
				if (profileIndex !== index) return profile;
				return {
					...profile,
					env: {...profile.env, [key]: value},
				};
			}),
		);
	};

	const removeProfileEnv = (index: number, key: string) => {
		setAcpProfiles((profiles) =>
			profiles.map((profile, profileIndex) => {
				if (profileIndex !== index) return profile;
				const nextEnv = {...profile.env};
				delete nextEnv[key];
				return {...profile, env: nextEnv};
			}),
		);
	};

	const removeProfile = (index: number) => {
		setAcpProfiles((profiles) => profiles.filter((_, i) => i !== index));
	};

	return (
		<div className="mx-auto max-w-3xl px-6 py-6">
			<div className="mb-6">
				<h2 className="font-plex text-sm font-semibold text-ink">
					Code Workers
				</h2>
				<p className="mt-1 text-sm text-ink-dull">
					Configure long-running coding workers backed by OpenCode or any
					ACP-compatible subprocess.
				</p>
			</div>

			<div className="mb-4 flex items-center gap-2 rounded-full border border-app-line/50 bg-app-dark-box/20 p-1">
				{(["opencode", "acp"] as WorkerTab[]).map((tab) => (
					<button
						key={tab}
						onClick={() => setActiveTab(tab)}
						className={
							activeTab === tab
								? "rounded-full bg-app-hover px-3 py-1 text-xs font-medium text-ink"
								: "rounded-full px-3 py-1 text-xs font-medium text-ink-faint hover:text-ink"
						}
					>
						{tab === "opencode" ? "OpenCode" : "ACP"}
					</button>
				))}
			</div>

			{message && (
				<div
					className={`mb-4 rounded-lg border px-4 py-3 text-sm ${
						message.type === "success"
							? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
							: "border-red-500/30 bg-red-500/10 text-red-300"
					}`}
				>
					{message.text}
				</div>
			)}

			{isLoading ? (
				<div className="flex items-center gap-2 text-ink-dull">
					<div className="h-2 w-2 animate-pulse rounded-full bg-accent" />
					Loading settings...
				</div>
			) : activeTab === "opencode" ? (
				<div className="flex flex-col gap-4">
					<div className="rounded-lg border border-app-line bg-app-box p-4">
						<label className="flex items-center gap-3">
							<input
								type="checkbox"
								checked={enabled}
								onChange={(e) => setEnabled(e.target.checked)}
								className="h-4 w-4"
							/>
							<div>
								<span className="text-sm font-medium text-ink">
									Enable OpenCode Workers
								</span>
								<p className="mt-0.5 text-sm text-ink-dull">
									Allow agents to spawn OpenCode coding sessions
								</p>
							</div>
						</label>
					</div>

					{enabled && (
						<>
							<div className="rounded-lg border border-app-line bg-app-box p-4">
								<label className="block">
									<span className="text-sm font-medium text-ink">Binary Path</span>
									<Input
										type="text"
										value={path}
										onChange={(e) => setPath(e.target.value)}
										placeholder="opencode"
										className="mt-2"
									/>
								</label>
							</div>
							<div className="rounded-lg border border-app-line bg-app-box p-4">
								<div className="grid grid-cols-3 gap-3">
									<label className="block">
										<span className="text-tiny font-medium text-ink-dull">Max Servers</span>
										<Input type="number" value={maxServers} onChange={(e) => setMaxServers(e.target.value)} className="mt-1" />
									</label>
									<label className="block">
										<span className="text-tiny font-medium text-ink-dull">Startup Timeout (s)</span>
										<Input type="number" value={startupTimeout} onChange={(e) => setStartupTimeout(e.target.value)} className="mt-1" />
									</label>
									<label className="block">
										<span className="text-tiny font-medium text-ink-dull">Max Retries</span>
										<Input type="number" value={maxRetries} onChange={(e) => setMaxRetries(e.target.value)} className="mt-1" />
									</label>
								</div>
							</div>
							<div className="rounded-lg border border-app-line bg-app-box p-4">
								<span className="text-sm font-medium text-ink">Permissions</span>
								<div className="mt-3 grid grid-cols-3 gap-3">
									{[
										["Edit", editPerm, setEditPerm],
										["Bash", bashPerm, setBashPerm],
										["Webfetch", webfetchPerm, setWebfetchPerm],
									].map(([label, value, setValue]) => (
										<label key={label} className="block">
											<span className="text-tiny font-medium text-ink-dull">{label}</span>
											<SelectRoot value={value as string} onValueChange={setValue as (value: string) => void}>
												<SelectTrigger className="mt-1">
													<SelectValue />
												</SelectTrigger>
												<SelectContent>
													{PERMISSION_OPTIONS.map((option) => (
														<SelectItem key={option.value} value={option.value}>
															{option.label}
														</SelectItem>
													))}
												</SelectContent>
											</SelectRoot>
										</label>
									))}
								</div>
							</div>
						</>
					)}
					<div className="flex justify-end">
						<Button onClick={saveOpenCode} disabled={updateMutation.isPending}>
							Save OpenCode
						</Button>
					</div>
				</div>
			) : (
				<div className="flex flex-col gap-4">
					<div className="rounded-lg border border-app-line bg-app-box p-4">
						<label className="flex items-center gap-3">
							<input
								type="checkbox"
								checked={acpEnabled}
								onChange={(e) => setAcpEnabled(e.target.checked)}
								className="h-4 w-4"
							/>
							<div>
								<span className="text-sm font-medium text-ink">
									Enable ACP Workers
								</span>
								<p className="mt-0.5 text-sm text-ink-dull">
									Allow agents to spawn external ACP-compatible binaries such as{" "}
									<code className="rounded bg-app-dark-box/40 px-1 py-0.5 text-tiny">
										claude acp
									</code>
									.
								</p>
							</div>
						</label>
					</div>

					<div className="rounded-lg border border-app-line bg-app-box p-4">
						<div className="grid grid-cols-2 gap-3">
							<label className="block">
								<span className="text-tiny font-medium text-ink-dull">
									Handshake Timeout (s)
								</span>
								<Input
									type="number"
									value={acpHandshakeTimeout}
									onChange={(e) => setAcpHandshakeTimeout(e.target.value)}
									className="mt-1"
								/>
							</label>
							<label className="block">
								<span className="text-tiny font-medium text-ink-dull">
									stderr Buffer (bytes)
								</span>
								<Input
									type="number"
									value={acpStderrBuffer}
									onChange={(e) => setAcpStderrBuffer(e.target.value)}
									className="mt-1"
								/>
							</label>
						</div>
						<p className="mt-3 text-xs text-ink-faint">
							ACP profile commands and env values support literal strings plus
							<code className="mx-1 rounded bg-app-dark-box/40 px-1 py-0.5">env:VAR</code>
							and
							<code className="mx-1 rounded bg-app-dark-box/40 px-1 py-0.5">secret:NAME</code>
							resolution in config.toml.
						</p>
					</div>

					<div className="rounded-lg border border-app-line bg-app-box p-4">
						<div className="mb-3 flex items-center justify-between">
							<span className="text-sm font-medium text-ink">Profiles</span>
							<Button variant="gray" size="sm" onClick={addProfile}>
								Add Profile
							</Button>
						</div>
						<div className="flex flex-col gap-4">
							{acpProfiles.map((profile, index) => (
								<div key={index} className="rounded-lg border border-app-line/60 bg-app-dark-box/10 p-4">
									<div className="mb-3 flex items-center justify-between">
										<span className="text-xs font-medium uppercase tracking-wider text-ink-faint">
											Profile {index + 1}
										</span>
										<button onClick={() => removeProfile(index)} className="text-xs text-red-300 hover:text-red-200">
											Remove
										</button>
									</div>
									<div className="grid grid-cols-2 gap-3">
										<Input
											value={profile.id}
											onChange={(e) => updateProfile(index, {id: e.target.value})}
											placeholder="claude"
										/>
										<Input
											value={profile.display_name ?? ""}
											onChange={(e) => updateProfile(index, {display_name: e.target.value})}
											placeholder="Claude Code"
										/>
										<Input
											value={profile.command}
											onChange={(e) => updateProfile(index, {command: e.target.value})}
											placeholder="claude"
										/>
										<textarea
											value={profileArgsToString(profile.args)}
											onChange={(e) => updateProfile(index, {args: parseArgs(e.target.value)})}
											placeholder={"acp\n--verbose"}
											rows={3}
											className="w-full rounded border border-app-line bg-app-input px-3 py-2 text-sm text-ink placeholder:text-ink-faint focus:outline-none focus:ring-1 focus:ring-accent/50"
										/>
									</div>
									<div className="mt-3 flex flex-col gap-2">
										<span className="text-tiny font-medium text-ink-dull">Environment</span>
										{Object.entries(profile.env).map(([key, value]) => (
											<div key={key} className="grid grid-cols-[1fr_1fr_auto] gap-2">
												<Input value={key} readOnly />
												<Input
													value={value}
													onChange={(e) => updateProfileEnv(index, key, e.target.value)}
													placeholder="secret:anthropic_api_key"
												/>
												<button onClick={() => removeProfileEnv(index, key)} className="text-xs text-red-300 hover:text-red-200">
													Remove
												</button>
											</div>
										))}
										<button
											onClick={() => updateProfileEnv(index, `ENV_${++envKeyCounter.current}`, "")}
											className="self-start text-xs text-accent hover:underline"
										>
											Add env var
										</button>
									</div>
								</div>
							))}
							{acpProfiles.length === 0 && (
								<div className="rounded-lg border border-dashed border-app-line/60 px-4 py-6 text-center text-sm text-ink-faint">
									No ACP profiles configured yet.
								</div>
							)}
						</div>
					</div>

					<div className="flex justify-end">
						<Button onClick={saveAcp} disabled={updateMutation.isPending}>
							Save ACP
						</Button>
					</div>
				</div>
			)}
		</div>
	);
}
