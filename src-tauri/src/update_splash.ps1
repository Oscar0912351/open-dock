$ErrorActionPreference = 'SilentlyContinue'
Add-Type -AssemblyName PresentationFramework,PresentationCore,WindowsBase

$stageFile    = '__STAGE_FILE__'
$theme        = '__THEME__'
$lang         = '__LANG__'
$versionBadge = '__VERSION_BADGE__'
$isLight      = ($theme -eq 'light')
$isEn         = ($lang -eq 'en')
$allowMotion  = $true
try { $allowMotion = [bool][System.Windows.SystemParameters]::ClientAreaAnimation } catch {}

function Make-Color([string]$hex) {
    [System.Windows.Media.ColorConverter]::ConvertFromString($hex)
}
function Make-Brush([string]$hex) {
    $b = New-Object System.Windows.Media.SolidColorBrush (Make-Color $hex)
    $b.Freeze()
    $b
}
function Make-Linear([string]$from, [string]$to, [double]$x1, [double]$y1, [double]$x2, [double]$y2) {
    $g = New-Object System.Windows.Media.LinearGradientBrush
    $g.StartPoint = New-Object System.Windows.Point($x1, $y1)
    $g.EndPoint   = New-Object System.Windows.Point($x2, $y2)
    $null = $g.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color $from), 0.0)))
    $null = $g.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color $to), 1.0)))
    $g.Freeze()
    $g
}

$copy = if ($isEn) {
    @{
        Title       = 'Updating Open Dock'
        Closing     = 'Closing…'
        Installing  = 'Installing…'
        Relaunching = 'Relaunching…'
        Done        = 'Starting…'
    }
} else {
    @{
        Title       = 'Opdaterer Open Dock'
        Closing     = 'Lukker…'
        Installing  = 'Installerer…'
        Relaunching = 'Starter igen…'
        Done        = 'Åbner…'
    }
}

$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Width="316" Height="324"
        WindowStyle="None" ResizeMode="NoResize"
        AllowsTransparency="True" Background="Transparent"
        Topmost="True" ShowInTaskbar="False"
        WindowStartupLocation="CenterScreen"
        UseLayoutRounding="True" SnapsToDevicePixels="True"
        TextOptions.TextFormattingMode="Ideal"
        TextOptions.TextRenderingMode="ClearType">
  <Grid x:Name="RootMotion" Margin="28" Opacity="0" RenderTransformOrigin="0.5,0.5">
    <Grid.RenderTransform>
      <TransformGroup>
        <ScaleTransform x:Name="RootScale" ScaleX="0.96" ScaleY="0.96"/>
        <TranslateTransform x:Name="RootTranslate" Y="6"/>
      </TransformGroup>
    </Grid.RenderTransform>

    <!-- Adskilt skygge-plade så al grafik og tekst forbliver knivskarp -->
    <Border x:Name="ShadowPlate" CornerRadius="28" IsHitTestVisible="False">
      <Border.Effect>
        <DropShadowEffect x:Name="CardShadow" Color="#000000" Direction="270"
                          ShadowDepth="12" BlurRadius="34" Opacity="0.65"/>
      </Border.Effect>
    </Border>

    <!-- Hovedflade: samme silhuet som Open Dock app-ikonet -->
    <Border x:Name="Card" CornerRadius="28" BorderThickness="1">
      <Grid x:Name="CardInner">
        <!-- Diskret Open Dock Studio dot-grid bag mærket -->
        <Rectangle x:Name="DotGrid" IsHitTestVisible="False"/>

        <!-- Meget rolig top-refleksion -->
        <Border x:Name="TopGlow" IsHitTestVisible="False"/>

        <!-- 1px specular top-kant -->
        <Rectangle x:Name="TopSpecular" Height="1" VerticalAlignment="Top"
                   Margin="30,0,30,0" IsHitTestVisible="False"/>

        <!-- Ét diskret entrance-sheen -->
        <Rectangle x:Name="Sheen" Width="110" HorizontalAlignment="Left"
                   IsHitTestVisible="False" Opacity="0">
          <Rectangle.RenderTransform>
            <TransformGroup>
              <SkewTransform AngleX="-22"/>
              <TranslateTransform x:Name="SheenX" X="-160"/>
            </TransformGroup>
          </Rectangle.RenderTransform>
        </Rectangle>

        <!-- Centreret indhold -->
        <Grid Margin="28,42,28,22">
          <Grid.RowDefinitions>
            <RowDefinition Height="*"/>
            <RowDefinition Height="Auto"/>
            <RowDefinition Height="Auto"/>
            <RowDefinition Height="Auto"/>
          </Grid.RowDefinitions>

          <!-- Open Dock 3-panel bento-mærke, fritstående og levende -->
          <Canvas Grid.Row="0" Width="68" Height="66"
                  HorizontalAlignment="Center" VerticalAlignment="Center">
            <Rectangle x:Name="TileA" Canvas.Left="0" Canvas.Top="0"
                       Width="24.5" Height="29" RadiusX="7" RadiusY="7">
              <Rectangle.RenderTransform>
                <TranslateTransform x:Name="TileAY" Y="0"/>
              </Rectangle.RenderTransform>
            </Rectangle>
            <Rectangle x:Name="TileB" Canvas.Left="31.5" Canvas.Top="0"
                       Width="36.5" Height="29" RadiusX="7" RadiusY="7">
              <Rectangle.RenderTransform>
                <TranslateTransform x:Name="TileBY" Y="0"/>
              </Rectangle.RenderTransform>
            </Rectangle>
            <Rectangle x:Name="TileC" Canvas.Left="0" Canvas.Top="36"
                       Width="68" Height="30" RadiusX="7" RadiusY="7">
              <Rectangle.RenderTransform>
                <TranslateTransform x:Name="TileCY" Y="0"/>
              </Rectangle.RenderTransform>
            </Rectangle>
          </Canvas>

          <!-- Titel og én rolig statuslinje -->
          <StackPanel Grid.Row="1" Margin="0,24,0,0" HorizontalAlignment="Center">
            <TextBlock x:Name="TitleText"
                       FontFamily="Segoe UI Variable Display, Segoe UI"
                       FontSize="14.5" FontWeight="SemiBold"
                       HorizontalAlignment="Center"/>
            <TextBlock x:Name="StageText" Margin="0,5,0,0"
                       FontFamily="Segoe UI Variable Text, Segoe UI"
                       FontSize="12" HorizontalAlignment="Center"/>
          </StackPanel>

          <!-- Hårfin 2.5px fremgangslinje -->
          <Grid x:Name="TrackHost" Grid.Row="2" Width="148" Height="2.5"
                Margin="0,22,0,0" HorizontalAlignment="Center">
            <Border x:Name="TrackBg" CornerRadius="1.25"/>
            <Border x:Name="TrackFill" HorizontalAlignment="Left" Width="14" CornerRadius="1.25"/>
          </Grid>

          <!-- Ren versionstekst uden ramme eller pill -->
          <TextBlock x:Name="VersionText" Grid.Row="3" Margin="0,14,0,0"
                     FontFamily="Segoe UI Variable Text, Segoe UI"
                     FontSize="10.5" HorizontalAlignment="Center"/>
        </Grid>
      </Grid>
    </Border>
  </Grid>
</Window>
'@

$reader = New-Object System.Xml.XmlNodeReader ([xml]$xaml)
$win = [System.Windows.Markup.XamlReader]::Load($reader)

function El([string]$name) { $win.FindName($name) }

$rootMotion    = El 'RootMotion'
$rootScale     = El 'RootScale'
$rootTranslate = El 'RootTranslate'
$shadowPlate   = El 'ShadowPlate'
$cardShadow    = El 'CardShadow'
$card          = El 'Card'
$cardInner     = El 'CardInner'
$dotGrid       = El 'DotGrid'
$topGlow       = El 'TopGlow'
$topSpecular   = El 'TopSpecular'
$sheen         = El 'Sheen'
$sheenX        = El 'SheenX'
$tileA         = El 'TileA'
$tileB         = El 'TileB'
$tileC         = El 'TileC'
$tileAY        = El 'TileAY'
$tileBY        = El 'TileBY'
$tileCY        = El 'TileCY'
$titleText     = El 'TitleText'
$stageText     = El 'StageText'
$trackHost     = El 'TrackHost'
$trackBg       = El 'TrackBg'
$trackFill     = El 'TrackFill'
$versionText   = El 'VersionText'

if ($isLight) {
    $pal = @{
        CardBg      = Make-Linear '#FDFFFFFF' '#FBF5F5F7' 0 0 0 1
        ShadowBg    = Make-Brush '#FFF8F8FA'
        ShadowOp    = 0.16
        ShadowBlur  = 28.0
        ShadowDepth = 9.0
        CardBorder  = Make-Brush '#1609090B'
        TileFill    = Make-Brush '#FF09090B'
        Ink1        = Make-Brush '#FF09090B'
        Ink2        = Make-Brush '#FF62626C'
        Ink3        = Make-Brush '#FF9E9EA8'
        TrackBg     = Make-Brush '#1609090B'
        TrackFill   = Make-Brush '#FF09090B'
        DotHex      = '#1809090B'
        GlowHex     = '#0809090B'
        SpecHex     = '#E6FFFFFF'
        SheenHex    = '#1AFFFFFF'
    }
} else {
    $pal = @{
        CardBg      = Make-Linear '#FA0D0D11' '#FC070709' 0 0 0 1
        ShadowBg    = Make-Brush '#FF070709'
        ShadowOp    = 0.66
        ShadowBlur  = 34.0
        ShadowDepth = 12.0
        CardBorder  = Make-Brush '#1CFFFFFF'
        TileFill    = Make-Brush '#FFF4F4F6'
        Ink1        = Make-Brush '#FFF4F4F6'
        Ink2        = Make-Brush '#FF868692'
        Ink3        = Make-Brush '#FF4C4C56'
        TrackBg     = Make-Brush '#18FFFFFF'
        TrackFill   = Make-Brush '#FFF4F4F6'
        DotHex      = '#22FFFFFF'
        GlowHex     = '#0CFFFFFF'
        SpecHex     = '#38FFFFFF'
        SheenHex    = '#12FFFFFF'
    }
}

$shadowPlate.Background = $pal.ShadowBg
$cardShadow.Opacity     = $pal.ShadowOp
$cardShadow.BlurRadius  = $pal.ShadowBlur
$cardShadow.ShadowDepth = $pal.ShadowDepth

$card.Background  = $pal.CardBg
$card.BorderBrush = $pal.CardBorder

$tileA.Fill = $pal.TileFill
$tileB.Fill = $pal.TileFill
$tileC.Fill = $pal.TileFill

# Studio dot-grid med blød radial maske centreret bag brikkerne
$dotGeom = New-Object System.Windows.Media.EllipseGeometry((New-Object System.Windows.Point(8, 8)), 0.75, 0.75)
$dotDrawing = New-Object System.Windows.Media.GeometryDrawing((Make-Brush $pal.DotHex), $null, $dotGeom)
$dotBrush = New-Object System.Windows.Media.DrawingBrush($dotDrawing)
$dotBrush.Viewbox       = New-Object System.Windows.Rect(0, 0, 16, 16)
$dotBrush.ViewboxUnits  = [System.Windows.Media.BrushMappingMode]::Absolute
$dotBrush.Viewport      = New-Object System.Windows.Rect(0, 0, 16, 16)
$dotBrush.ViewportUnits = [System.Windows.Media.BrushMappingMode]::Absolute
$dotBrush.TileMode      = [System.Windows.Media.TileMode]::Tile
$dotBrush.Freeze()
$dotGrid.Fill = $dotBrush

$dotMask = New-Object System.Windows.Media.RadialGradientBrush
$dotMask.GradientOrigin = New-Object System.Windows.Point(0.5, 0.36)
$dotMask.Center         = New-Object System.Windows.Point(0.5, 0.36)
$dotMask.RadiusX        = 0.52
$dotMask.RadiusY        = 0.46
$null = $dotMask.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#FFFFFFFF'), 0.0)))
$null = $dotMask.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#00FFFFFF'), 1.0)))
$dotMask.Freeze()
$dotGrid.OpacityMask = $dotMask

# Subtil top-glød
$glow = New-Object System.Windows.Media.RadialGradientBrush
$glow.GradientOrigin = New-Object System.Windows.Point(0.5, 0.0)
$glow.Center         = New-Object System.Windows.Point(0.5, 0.0)
$glow.RadiusX        = 0.75
$glow.RadiusY        = 0.55
$null = $glow.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color $pal.GlowHex), 0.0)))
$null = $glow.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#00FFFFFF'), 1.0)))
$glow.Freeze()
$topGlow.Background = $glow

function Make-SpecBrush([string]$midHex) {
    $b = New-Object System.Windows.Media.LinearGradientBrush
    $b.StartPoint = New-Object System.Windows.Point(0, 0.5)
    $b.EndPoint   = New-Object System.Windows.Point(1, 0.5)
    $null = $b.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#00FFFFFF'), 0.0)))
    $null = $b.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color $midHex), 0.5)))
    $null = $b.GradientStops.Add((New-Object System.Windows.Media.GradientStop((Make-Color '#00FFFFFF'), 1.0)))
    $b.Freeze()
    $b
}
$topSpecular.Fill = Make-SpecBrush $pal.SpecHex
$sheen.Fill       = Make-SpecBrush $pal.SheenHex

$titleText.Text         = $copy.Title
$titleText.Foreground   = $pal.Ink1
$stageText.Foreground   = $pal.Ink2
$versionText.Foreground = $pal.Ink3
if ([string]::IsNullOrWhiteSpace($versionBadge)) {
    $versionText.Visibility = [System.Windows.Visibility]::Collapsed
} else {
    $versionText.Text = $versionBadge
}

$trackBg.Background   = $pal.TrackBg
$trackFill.Background = $pal.TrackFill

$cardInner.Add_SizeChanged({
    $rect = New-Object System.Windows.Rect(0, 0, $cardInner.ActualWidth, $cardInner.ActualHeight)
    $cardInner.Clip = New-Object System.Windows.Media.RectangleGeometry($rect, 27, 27)
})
$trackHost.Add_SizeChanged({
    $rect = New-Object System.Windows.Rect(0, 0, $trackHost.ActualWidth, $trackHost.ActualHeight)
    $trackHost.Clip = New-Object System.Windows.Media.RectangleGeometry($rect, 1.25, 1.25)
})

$win.Add_MouseLeftButtonDown({
    try { $win.DragMove() } catch {}
})

$easeOut = New-Object System.Windows.Media.Animation.CubicEase
$easeOut.EasingMode = [System.Windows.Media.Animation.EasingMode]::EaseOut
$easeInOut = New-Object System.Windows.Media.Animation.SineEase
$easeInOut.EasingMode = [System.Windows.Media.Animation.EasingMode]::EaseInOut

$script:currentStage = ''
$script:progressFrac = 0.08
$script:closingOut   = $false

function Animate-Fill([double]$frac, [int]$ms) {
    $script:progressFrac = $frac
    $w = $trackHost.ActualWidth
    if ($w -le 0) { $w = 148.0 }
    $targetW = [Math]::Max(8.0, $w * $frac)
    if (-not $allowMotion -or $ms -le 0) {
        $trackFill.BeginAnimation([System.Windows.FrameworkElement]::WidthProperty, $null)
        $trackFill.Width = $targetW
        return
    }
    $anim = New-Object System.Windows.Media.Animation.DoubleAnimation
    $anim.To = $targetW
    $anim.Duration = [System.Windows.Duration]::new([TimeSpan]::FromMilliseconds($ms))
    $anim.EasingFunction = $easeOut
    $trackFill.BeginAnimation([System.Windows.FrameworkElement]::WidthProperty, $anim)
}

function Start-AmbientAnimations {
    if (-not $allowMotion) { return }

    # De 3 brikker løfter sig og ånder stille i sekvens
    $tiles  = @($tileA, $tileB, $tileC)
    $shifts = @($tileAY, $tileBY, $tileCY)
    for ($i = 0; $i -lt 3; $i++) {
        $op = New-Object System.Windows.Media.Animation.DoubleAnimation
        $op.From = 1.0
        $op.To   = 0.54
        $op.BeginTime = [TimeSpan]::FromMilliseconds($i * 250)
        $op.Duration  = [System.Windows.Duration]::new([TimeSpan]::FromMilliseconds(950))
        $op.AutoReverse = $true
        $op.RepeatBehavior = [System.Windows.Media.Animation.RepeatBehavior]::Forever
        $op.EasingFunction = $easeInOut
        $tiles[$i].BeginAnimation([System.Windows.UIElement]::OpacityProperty, $op)

        $dy = New-Object System.Windows.Media.Animation.DoubleAnimation
        $dy.From = 0.0
        $dy.To   = -3.0
        $dy.BeginTime = [TimeSpan]::FromMilliseconds($i * 250)
        $dy.Duration  = [System.Windows.Duration]::new([TimeSpan]::FromMilliseconds(950))
        $dy.AutoReverse = $true
        $dy.RepeatBehavior = [System.Windows.Media.Animation.RepeatBehavior]::Forever
        $dy.EasingFunction = $easeInOut
        $shifts[$i].BeginAnimation([System.Windows.Media.TranslateTransform]::YProperty, $dy)
    }

    # Ét enkelt stille lys-sheen ved entrance
    $sheen.Opacity = 1.0
    $sh = New-Object System.Windows.Media.Animation.DoubleAnimation
    $sh.From = -160.0
    $sh.To   = 420.0
    $sh.BeginTime = [TimeSpan]::FromMilliseconds(160)
    $sh.Duration  = [System.Windows.Duration]::new([TimeSpan]::FromMilliseconds(850))
    $sh.EasingFunction = $easeOut
    $sheenX.BeginAnimation([System.Windows.Media.TranslateTransform]::XProperty, $sh)
}

function Apply-Stage([string]$stage) {
    $norm = ($stage -replace '\s+', '').ToLowerInvariant()
    if ([string]::IsNullOrEmpty($norm)) { $norm = 'closing' }
    if ($norm -eq $script:currentStage) { return }
    $script:currentStage = $norm

    switch ($norm) {
        'closing' {
            $stageText.Text = $copy.Closing
            Animate-Fill 0.22 460
        }
        'installing' {
            $stageText.Text = $copy.Installing
            Animate-Fill 0.62 900
        }
        'relaunching' {
            $stageText.Text = $copy.Relaunching
            Animate-Fill 0.90 480
        }
        'done' {
            $stageText.Text = $copy.Done
            Animate-Fill 1.0 200

            foreach ($t in @($tileA, $tileB, $tileC)) {
                $t.BeginAnimation([System.Windows.UIElement]::OpacityProperty, $null)
                $t.Opacity = 1.0
            }
            foreach ($s in @($tileAY, $tileBY, $tileCY)) {
                $s.BeginAnimation([System.Windows.Media.TranslateTransform]::YProperty, $null)
                $s.Y = 0.0
            }

            if (-not $script:closingOut) {
                $script:closingOut = $true
                $closeDelay = New-Object System.Windows.Threading.DispatcherTimer
                $closeDelay.Interval = [TimeSpan]::FromMilliseconds(320)
                $closeDelay.Add_Tick({
                    $closeDelay.Stop()
                    if (-not $allowMotion) {
                        $win.Close()
                        return
                    }
                    $fadeOut = New-Object System.Windows.Media.Animation.DoubleAnimation(1.0, 0.0, [TimeSpan]::FromMilliseconds(220))
                    $fadeOut.EasingFunction = $easeOut
                    $fadeOut.Add_Completed({ $win.Close() })
                    $scaleOut = New-Object System.Windows.Media.Animation.DoubleAnimation(1.0, 0.96, [TimeSpan]::FromMilliseconds(220))
                    $scaleOut.EasingFunction = $easeOut
                    $rootMotion.BeginAnimation([System.Windows.UIElement]::OpacityProperty, $fadeOut)
                    $rootScale.BeginAnimation([System.Windows.Media.ScaleTransform]::ScaleXProperty, $scaleOut)
                    $rootScale.BeginAnimation([System.Windows.Media.ScaleTransform]::ScaleYProperty, $scaleOut)
                })
                $closeDelay.Start()
            }
        }
        default {
            $stageText.Text = $copy.Installing
            Animate-Fill 0.55 700
        }
    }
}

$win.Add_Loaded({
    if ($allowMotion) {
        $op = New-Object System.Windows.Media.Animation.DoubleAnimation(0.0, 1.0, [TimeSpan]::FromMilliseconds(260))
        $op.EasingFunction = $easeOut
        $ty = New-Object System.Windows.Media.Animation.DoubleAnimation(6.0, 0.0, [TimeSpan]::FromMilliseconds(320))
        $ty.EasingFunction = $easeOut
        $sc = New-Object System.Windows.Media.Animation.DoubleAnimation(0.96, 1.0, [TimeSpan]::FromMilliseconds(320))
        $sc.EasingFunction = $easeOut
        $rootMotion.BeginAnimation([System.Windows.UIElement]::OpacityProperty, $op)
        $rootTranslate.BeginAnimation([System.Windows.Media.TranslateTransform]::YProperty, $ty)
        $rootScale.BeginAnimation([System.Windows.Media.ScaleTransform]::ScaleXProperty, $sc)
        $rootScale.BeginAnimation([System.Windows.Media.ScaleTransform]::ScaleYProperty, $sc)
    } else {
        $rootMotion.Opacity = 1.0
        $rootTranslate.Y    = 0.0
        $rootScale.ScaleX   = 1.0
        $rootScale.ScaleY   = 1.0
    }

    Start-AmbientAnimations

    $initial = 'closing'
    if (Test-Path -LiteralPath $stageFile) {
        try { $initial = [System.IO.File]::ReadAllText($stageFile) } catch {}
    }
    Apply-Stage $initial
})

$poll = New-Object System.Windows.Threading.DispatcherTimer
$poll.Interval = [TimeSpan]::FromMilliseconds(180)
$poll.Add_Tick({
    if (Test-Path -LiteralPath $stageFile) {
        try {
            $txt = [System.IO.File]::ReadAllText($stageFile)
            Apply-Stage $txt
        } catch {}
    }
    if ($script:currentStage -eq 'installing' -and $script:progressFrac -lt 0.84) {
        $next = [Math]::Min(0.84, $script:progressFrac + 0.008)
        Animate-Fill $next 220
    }
})
$poll.Start()

$life = New-Object System.Windows.Threading.DispatcherTimer
$life.Interval = [TimeSpan]::FromMinutes(3)
$life.Add_Tick({ $win.Close() })
$life.Start()

$null = $win.ShowDialog()
