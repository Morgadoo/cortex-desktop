/** Common emoji by shortcode, for `:name` in the composer (the names GitHub and Slack use). */
const EMOJI: Record<string, string> = {
  smile: '😄', grin: '😁', joy: '😂', rofl: '🤣', slightly_smiling_face: '🙂', wink: '😉', blush: '😊', heart_eyes: '😍',
  thinking: '🤔', neutral_face: '😐', expressionless: '😑', unamused: '😒', roll_eyes: '🙄', grimacing: '😬', relieved: '😌',
  pensive: '😔', sleepy: '😪', sleeping: '😴', mask: '😷', nerd_face: '🤓', sunglasses: '😎', confused: '😕', worried: '😟',
  open_mouth: '😮', astonished: '😲', flushed: '😳', pleading_face: '🥺', cry: '😢', sob: '😭', scream: '😱', angry: '😠',
  rage: '😡', exploding_head: '🤯', partying_face: '🥳', zany_face: '🤪', shushing_face: '🤫', hugs: '🤗', upside_down_face: '🙃',
  smirk: '😏', innocent: '😇', skull: '💀', ghost: '👻', robot: '🤖', alien: '👽', clown_face: '🤡', poop: '💩',
  thumbsup: '👍', '+1': '👍', thumbsdown: '👎', '-1': '👎', ok_hand: '👌', wave: '👋', clap: '👏', raised_hands: '🙌',
  pray: '🙏', handshake: '🤝', muscle: '💪', point_up: '☝️', point_right: '👉', point_left: '👈', point_down: '👇',
  crossed_fingers: '🤞', v: '✌️', metal: '🤘', call_me_hand: '🤙', writing_hand: '✍️', eyes: '👀', brain: '🧠',
  heart: '❤️', orange_heart: '🧡', yellow_heart: '💛', green_heart: '💚', blue_heart: '💙', purple_heart: '💜',
  black_heart: '🖤', broken_heart: '💔', sparkling_heart: '💖', fire: '🔥', sparkles: '✨', star: '⭐', star2: '🌟',
  zap: '⚡', boom: '💥', '100': '💯', tada: '🎉', confetti_ball: '🎊', gift: '🎁', trophy: '🏆', medal: '🏅',
  rocket: '🚀', bulb: '💡', memo: '📝', pencil2: '✏️', book: '📖', books: '📚', bookmark: '🔖', link: '🔗', paperclip: '📎',
  pushpin: '📌', calendar: '📅', clock: '🕐', hourglass: '⏳', alarm_clock: '⏰', lock: '🔒', unlock: '🔓', key: '🔑',
  bell: '🔔', mag: '🔍', gear: '⚙️', wrench: '🔧', hammer: '🔨', hammer_and_wrench: '🛠️', toolbox: '🧰', package: '📦',
  computer: '💻', keyboard: '⌨️', iphone: '📱', email: '📧', inbox_tray: '📥', outbox_tray: '📤', chart_with_upwards_trend: '📈',
  chart_with_downwards_trend: '📉', bar_chart: '📊', clipboard: '📋', file_folder: '📁', card_index: '📇', bug: '🐛',
  white_check_mark: '✅', heavy_check_mark: '✔️', x: '❌', warning: '⚠️', no_entry: '⛔', question: '❓', exclamation: '❗',
  information_source: 'ℹ️', recycle: '♻️', construction: '🚧', rotating_light: '🚨', red_circle: '🔴', green_circle: '🟢',
  yellow_circle: '🟡', blue_circle: '🔵', arrow_right: '➡️', arrow_left: '⬅️', arrow_up: '⬆️', arrow_down: '⬇️',
  coffee: '☕', tea: '🍵', beer: '🍺', wine_glass: '🍷', pizza: '🍕', hamburger: '🍔', cake: '🍰', apple: '🍎',
  sun: '☀️', sunny: '☀️', cloud: '☁️', rain: '🌧️', snowflake: '❄️', rainbow: '🌈', earth_africa: '🌍', earth_americas: '🌎',
  globe_with_meridians: '🌐', seedling: '🌱', evergreen_tree: '🌲', rose: '🌹', sunflower: '🌻', cat: '🐱', dog: '🐶',
  crab: '🦀', snake: '🐍', unicorn: '🦄', bee: '🐝', owl: '🦉', moneybag: '💰', dollar: '💵', euro: '💶', gem: '💎',
  house: '🏠', office: '🏢', airplane: '✈️', car: '🚗', train: '🚆', world_map: '🗺️', flag_pt: '🇵🇹', checkered_flag: '🏁',
  speech_balloon: '💬', thought_balloon: '💭', zzz: '💤', sweat_smile: '😅', sweat: '😓', laughing: '😆', star_struck: '🤩',
  money_mouth_face: '🤑', facepalm: '🤦', shrug: '🤷', ok: '🆗', new: '🆕', free: '🆓', top: '🔝', soon: '🔜',
};

/** Shortcodes that start with (then contain) `q`, at most `max`, as [name, emoji]. */
export function findEmoji(q: string, max = 8): [string, string][] {
  const query = q.toLowerCase();
  const names = Object.keys(EMOJI);
  const starts = names.filter((n) => n.startsWith(query));
  const contains = names.filter((n) => !n.startsWith(query) && n.includes(query));
  return [...starts, ...contains].slice(0, max).map((n) => [n, EMOJI[n]!]);
}

/** The emoji for an exact shortcode, if there is one. */
export function emojiFor(name: string): string | undefined {
  return EMOJI[name.toLowerCase()];
}
