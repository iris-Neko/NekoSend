import 'package:flutter/material.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../application/models.dart';

typedef AvatarStyle = ({
  String name,
  IconData icon,
  Color background,
  Color ink,
});

const Map<String, AvatarStyle> builtinAvatarStyles = {
  'cat': (
    name: '小猫',
    icon: LucideIcons.cat,
    background: Color(0xFFFFE6BB),
    ink: Color(0xFF865315),
  ),
  'dog': (
    name: '小狗',
    icon: LucideIcons.dog,
    background: Color(0xFFFCDAD7),
    ink: Color(0xFF974B46),
  ),
  'rabbit': (
    name: '兔子',
    icon: LucideIcons.rabbit,
    background: Color(0xFFF4DEEF),
    ink: Color(0xFF914875),
  ),
  'bird': (
    name: '飞鸟',
    icon: LucideIcons.bird,
    background: Color(0xFFD9EDF9),
    ink: Color(0xFF316F93),
  ),
  'fish': (
    name: '小鱼',
    icon: LucideIcons.fish,
    background: Color(0xFFD4F1EB),
    ink: Color(0xFF267764),
  ),
  'bot': (
    name: '机器人',
    icon: LucideIcons.bot,
    background: Color(0xFFE3E7F0),
    ink: Color(0xFF53648E),
  ),
  'rocket': (
    name: '火箭',
    icon: LucideIcons.rocket,
    background: Color(0xFFFADDD0),
    ink: Color(0xFFAD573E),
  ),
  'flower': (
    name: '花朵',
    icon: LucideIcons.flower2,
    background: Color(0xFFEADFF5),
    ink: Color(0xFF795094),
  ),
  'mountain': (
    name: '山峰',
    icon: LucideIcons.mountain,
    background: Color(0xFFDEECD4),
    ink: Color(0xFF587637),
  ),
  'coffee': (
    name: '咖啡',
    icon: LucideIcons.coffee,
    background: Color(0xFFF0E1D5),
    ink: Color(0xFF846047),
  ),
  'moon': (
    name: '月亮',
    icon: LucideIcons.moonStar,
    background: Color(0xFFDDE4F7),
    ink: Color(0xFF536FAD),
  ),
  'sun': (
    name: '太阳',
    icon: LucideIcons.sun,
    background: Color(0xFFFFF0B8),
    ink: Color(0xFF9B7616),
  ),
};

class DeviceAvatar extends StatelessWidget {
  const DeviceAvatar({
    super.key,
    required this.deviceId,
    this.avatarId,
    this.size = 38,
    this.semanticLabel,
  });

  final String deviceId;
  final String? avatarId;
  final double size;
  final String? semanticLabel;

  @override
  Widget build(BuildContext context) {
    final style =
        builtinAvatarStyles[avatarId] ??
        builtinAvatarStyles[defaultAvatarId(deviceId)]!;
    return Semantics(
      image: true,
      label: semanticLabel ?? '${style.name}头像',
      child: Container(
        width: size,
        height: size,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: style.background,
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: style.ink.withValues(alpha: 0.12)),
        ),
        child: ExcludeSemantics(
          child: Icon(style.icon, color: style.ink, size: size * 0.61),
        ),
      ),
    );
  }
}
